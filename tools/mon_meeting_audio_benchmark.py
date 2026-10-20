#!/usr/bin/env python3
"""Measure one whole-WAV sherpa call; this is not an end-to-end/accuracy test."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time
import wave


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def make_wave(source, target, seconds, repeat):
    """Write bounded blocks, retaining the input's sample order exactly."""
    with wave.open(str(source), 'rb') as src:
        if (src.getframerate(), src.getnchannels(), src.getsampwidth(), src.getcomptype()) != (16000, 1, 2, 'NONE'):
            raise ValueError('input must be mono PCM16 16 kHz WAV')
        frames = seconds * 16000
        if src.getnframes() == 0 or frames > src.getnframes() and not repeat:
            raise ValueError('input is too short; --repeat-input explicitly enables synthetic repetition')
        if frames * 2 > 0xFFFFFFFF - 36:
            raise ValueError('WAV size limit exceeded')
        with target.open('xb') as file:
            with wave.open(file, 'wb') as dst:
                dst.setparams((1, 2, 16000, frames, 'NONE', 'not compressed'))
                remaining = frames
                while remaining:
                    block = src.readframes(min(remaining, 65536))
                    if not block:
                        if not repeat or src.tell() < src.getnframes():
                            raise ValueError('truncated input WAV')
                        src.rewind()
                        block = src.readframes(min(remaining, 65536))
                    if not block or len(block) % 2:
                        raise ValueError('truncated input WAV')
                    dst.writeframesraw(block)
                    remaining -= len(block) // 2
            file.flush()
            os.fsync(file.fileno())


def infer(worker, directory, args, timeout):
    log = (directory / 'worker.log').open('x')
    for name in ['stdout.log', 'stderr.log']:
        (directory / name).touch(exist_ok=False)
    process = subprocess.Popen([str(worker), '--mon-resident-v1'], stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=log, start_new_session=True,
                               env={'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'LC_ALL': 'C'})
    pending = b''
    deadline = time.monotonic() + timeout

    def send(value):
        process.stdin.write((json.dumps(value) + '\n').encode())
        process.stdin.flush()

    def receive(expected):
        nonlocal pending
        while b'\n' not in pending:
            if time.monotonic() >= deadline:
                raise TimeoutError('native worker timed out')
            if not select.select([process.stdout], [], [], 1)[0]:
                continue
            block = os.read(process.stdout.fileno(), 65536)
            if not block:
                raise RuntimeError('worker exited; inspect stderr.log and worker.log')
            pending += block
            if len(pending) > 65536:
                raise RuntimeError('worker response exceeds protocol bound')
        line, pending = pending.split(b'\n', 1)
        value = json.loads(line)
        if value.get('protocol') != 1 or value.get('id') != expected:
            raise RuntimeError('worker response identity mismatch')
        if value.get('op') == 'error':
            raise RuntimeError(str(value.get('message')))
        return value

    try:
        send(dict(protocol=1, id=1, op='init'))
        if receive(1)['op'] != 'ready':
            raise RuntimeError('worker did not initialize')
        started = time.monotonic()
        send(dict(protocol=1, id=2, op='infer', kind='sherpa', directory=str(directory),
                  stdout='stdout.log', stderr='stderr.log', args=args))
        while receive(2)['op'] != 'done':
            pass
        seconds = time.monotonic() - started
        send(dict(protocol=1, id=3, op='shutdown'))
        process.stdin.close()
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        if process.returncode:
            raise RuntimeError(f'worker shutdown failed: {process.returncode}')
        peak = usage.ru_maxrss * (1 if sys.platform == 'darwin' else 1024)
        return seconds, peak
    finally:
        if process.returncode is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        process.stdout.close()
        if not process.stdin.closed:
            process.stdin.close()
        log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--sherpa-root', type=Path, required=True, help='pinned package containing models/ and lib/')
    parser.add_argument('--input', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='new directory; never overwrites an earlier run')
    parser.add_argument('--durations', type=int, nargs='+', default=[600, 3600, 14400])
    parser.add_argument('--threads', type=int, default=8)
    parser.add_argument('--num-speakers', default='5', help='1..100 or auto')
    parser.add_argument('--window-shift', type=float, default=0.1)
    parser.add_argument('--repeat-input', action='store_true')
    parser.add_argument('--timeout-seconds', type=int, default=86400)
    options = parser.parse_args()
    if not hasattr(os, 'wait4'):
        parser.error('Unix wait4 resource accounting is required')
    if not 1 <= options.threads <= 64 or not 0.001 <= options.window_shift <= 1:
        parser.error('invalid inference settings')
    count = -1 if options.num_speakers == 'auto' else int(options.num_speakers)
    if count != -1 and not 1 <= count <= 100 or any(d <= 0 for d in options.durations):
        parser.error('invalid speaker limit/duration')
    source, worker, models = options.input.resolve(strict=True), options.worker.resolve(strict=True), options.sherpa_root.resolve(strict=True)
    output = options.output.absolute()
    output.mkdir(parents=True, exist_ok=False)
    files = {'worker': worker, 'input': source, 'segmentation': models / 'models/segmentation.onnx',
             'embedding': models / 'models/embedding.onnx', 'library': models / 'lib/libsherpa-onnx-c-api.dylib'}
    report = dict(scope='native_sherpa_only', accuracyEvaluated=False, repeatedInput=options.repeat_input,
                  threads=options.threads, numClusters=count, windowShift=options.window_shift,
                  hashes={name: digest(path) for name, path in files.items()}, cases=[])
    try:
        for index, duration in enumerate(options.durations):
            directory = output / f'{index}-{duration}s'
            directory.mkdir()
            print(f'Preparing {duration}s WAV', flush=True)
            make_wave(source, directory / 'input.wav', duration, options.repeat_input)
            args = [f'--segmentation.pyannote-model={files["segmentation"]}', f'--embedding.model={files["embedding"]}',
                    f'--segmentation.num-threads={options.threads}', f'--embedding.num-threads={options.threads}',
                    f'--segmentation.pyannote-window-shift-ratio={options.window_shift}', f'--clustering.num-clusters={count}',
                    '--clustering.cluster-threshold=0.5', '--clustering.compute-confidence=false',
                    '--min-duration-on=0.3', '--min-duration-off=0.5', str(directory / 'input.wav'), '--library', str(files['library'])]
            print(f'Running one native call over {duration}s', flush=True)
            elapsed, peak = infer(worker, directory, args, options.timeout_seconds)
            row = dict(durationSeconds=duration, inferenceSeconds=elapsed, peakWorkerResidentBytes=peak,
                       inputHash=digest(directory / 'input.wav'), nativeOutputHash=digest(directory / 'stdout.log'))
            report['cases'].append(row)
            print(json.dumps(row), flush=True)
            temporary = output / 'report.pending'
            temporary.write_text(json.dumps(report, indent=2) + '\n')
            os.replace(temporary, output / 'report.json')
    except BaseException as error:
        (output / 'failure.txt').write_text(str(error) + '\n')
        raise


if __name__ == '__main__':
    main()
