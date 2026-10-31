#!/usr/bin/env python3
"""Run reproducible, local ASR smoke comparisons without modifying app data.

Haas: compare greedy, beam without hints and beam with explicitly supplied
spellings. Also check silence, a >29-second input, ignored-word diagnostics and
optional unhinted byte-for-byte ReazonSpeech regression against an existing
worker, plus the corrected tab-delimited hotword parsing path.
Windows peak working set/commit come from GetProcessMemoryInfo, sampled while
each fresh process runs. These cold-start smoke timings are not a benchmark.
"""
import argparse
import ctypes
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import time
import unicodedata
import wave


def digest(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def memory(process):
    if os.name != 'nt':
        return None
    from ctypes import wintypes

    class Counters(ctypes.Structure):
        _fields_ = [('cb', wintypes.DWORD), ('faults', wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in (
                'peak_rss', 'rss', 'peak_paged', 'paged', 'peak_nonpaged',
                'nonpaged', 'commit', 'peak_commit', 'private')]

    query = ctypes.windll.psapi.GetProcessMemoryInfo
    query.argtypes = (wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD)
    query.restype = wintypes.BOOL
    counters = Counters()
    counters.cb = ctypes.sizeof(counters)
    if not query(wintypes.HANDLE(int(process._handle)), ctypes.byref(counters), counters.cb):
        raise ctypes.WinError()
    return counters.peak_rss, counters.peak_commit


def normalized(text):
    return ''.join(c for c in unicodedata.normalize('NFKC', text) if c.isalnum())


def cer(reference, actual):
    reference, actual = normalized(reference), normalized(actual)
    previous = list(range(len(actual) + 1))
    for i, expected in enumerate(reference, 1):
        current = [i]
        for j, found in enumerate(actual, 1):
            current.append(min(current[-1] + 1, previous[j] + 1,
                               previous[j - 1] + (expected != found)))
        previous = current
    return previous[-1] / max(1, len(reference))


def audio_info(path):
    with wave.open(str(path), 'rb') as source:
        if source.getnchannels() != 1 or source.getframerate() != 16000 or source.getsampwidth() != 2:
            raise ValueError('audio must be mono PCM16 / 16 kHz')
        return source.getnframes(), source.readframes(source.getnframes())


def write_audio(path, pcm):
    with wave.open(str(path), 'wb') as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(16000)
        output.writeframes(pcm)


def validate_result(result, duration):
    if result['schemaVersion'] != 1 or abs(result['durationUs'] - duration) > 100:
        raise ValueError('result duration/schema mismatch')
    previous = 0
    for segment in result['segments']:
        if not (previous <= segment['startUs'] < segment['endUs'] <= duration) or not segment['text'].strip():
            raise ValueError('invalid or overlapping transcript segment')
        previous = segment['endUs']
    previous = 0
    for token in result['tokens']:
        if not previous <= token['startUs'] < duration:
            raise ValueError('invalid token timestamp')
        previous = token['startUs']


def run_case(worker, library, assets, audio, directory, name, mode, hotwords='', haas=True, timeout=600):
    unit = directory / name
    unit.mkdir()
    frames, _ = audio_info(audio)
    members = ({'model': 'encoder.quantized.onnx', 'decoder': 'decoder.onnx',
                'joiner': 'joiner.onnx', 'tokens': 'tokens.txt', 'data': 'encoder.data',
                'bpeVocab': 'bpe.vocab'} if haas else
               {'model': 'encoder-epoch-99-avg-1.int8.onnx',
                'decoder': 'decoder-epoch-99-avg-1.onnx',
                'joiner': 'joiner-epoch-99-avg-1.int8.onnx', 'tokens': 'tokens.txt'})
    arguments = [str(worker), '--library', str(library), '--file', str(audio),
                 '--mon-first-frame', '0', '--mon-end-frame', str(frames),
                 '--output', str(unit / 'result.json'), '--threads', '4',
                 '--hotwords', hotwords, '--vad-enabled', '0']
    for key, relative in members.items():
        arguments += ['--' + key, str(assets / relative)]
    if haas:
        arguments += ['--backend-id', 'sherpa-onnx-nemo-tdt', '--model-type', 'nemo_transducer',
                      '--feature-dim', '128', '--modeling-unit', 'bpe', '--max-segment-seconds', '29']
        if mode:
            arguments += ['--decoding-method', mode]
    started = time.perf_counter()
    peaks = (0, 0)
    with (unit / 'stdout.log').open('wb') as stdout, (unit / 'stderr.log').open('wb') as stderr:
        process = subprocess.Popen(arguments, stdout=stdout, stderr=stderr)
        try:
            while process.poll() is None:
                observed = memory(process)
                if observed:
                    peaks = tuple(max(before, after) for before, after in zip(peaks, observed))
                if time.perf_counter() - started > timeout:
                    raise TimeoutError('native ASR evaluation exceeded timeout')
                time.sleep(.025)
            # Windows retains the peak counters on the process handle after exit.
            observed = memory(process)
            if observed:
                peaks = tuple(max(before, after) for before, after in zip(peaks, observed))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
    elapsed = time.perf_counter() - started
    if process.returncode:
        raise RuntimeError(f'{name}: native exit {process.returncode}; see {unit / "stderr.log"}')
    result = json.loads((unit / 'result.json').read_text(encoding='utf8'))
    duration = frames * 1_000_000 // 16000
    validate_result(result, duration)
    expected_backend = 'sherpa-onnx-nemo-tdt' if haas else 'sherpa-onnx-transducer'
    if result['backendId'] != expected_backend:
        raise ValueError('wrong backend result')
    expected_models = dict(library=str(library), **{key: str(assets / value) for key, value in members.items()})
    if result['models'] != expected_models:
        raise ValueError('wrong model result bindings')
    logs = (unit / 'stderr.log').read_text(encoding='utf8', errors='replace')
    ranges = [[int(a), int(b)] for a, b in re.findall(r'mon_asr_range: first_frame=(\d+) end_frame=(\d+)', logs)]
    if haas and (not ranges or any(b - a > 29 * 16000 for a, b in ranges)):
        raise ValueError('Haas segment limit not enforced')
    transcript = ''.join(segment['text'] for segment in result['segments'])
    summary = dict(name=name, decodingMethod=mode or 'auto', threads=4, vadEnabled=False,
                   audioFrames=frames, audioSeconds=frames / 16000, elapsedSeconds=elapsed,
                   rtf=elapsed / (frames / 16000), peakWorkingSetBytes=peaks[0] if os.name == 'nt' else None,
                   peakCommitBytes=peaks[1] if os.name == 'nt' else None,
                   text=transcript, segments=len(result['segments']), tokens=len(result['tokens']),
                   ignoredHotwords=result['ignoredHotwords'],
                   ignoredHotwordReasons=result.get('ignoredHotwordReasons', {}), ranges=ranges)
    return summary, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--library', type=Path, required=True)
    parser.add_argument('--assets', type=Path, required=True)
    parser.add_argument('--audio', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--hotword', action='append', required=True,
                        help='Spelling to boost; repeat for multiple entries, choose before looking at results.')
    parser.add_argument('--reference', help='Optional known verbatim transcript for plain NFKC/alphanumeric CER.')
    parser.add_argument('--legacy-worker', type=Path)
    parser.add_argument('--reazon-assets', type=Path)
    parser.add_argument('--comparison-only', action='store_true',
                        help='Run only the three decoding comparisons; omit supplemental smoke checks.')
    args = parser.parse_args()
    if args.comparison_only and args.legacy_worker:
        parser.error('--comparison-only cannot be combined with --legacy-worker')
    worker, library, assets, audio = (path.resolve(strict=True) for path in
                                      (args.worker, args.library, args.assets, args.audio))
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = dict(platform=os.name, evidence='cold-start smoke; not a controlled benchmark',
                  workerSha256=digest(worker), librarySha256=digest(library),
                  audioSha256=digest(audio), hotwords=args.hotword, reference=args.reference, cases=[])

    def save():
        (output / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf8')

    def case(name, mode, hotwords='', source=audio, **kwargs):
        summary, result = run_case(worker, library, assets, source, output, name, mode, hotwords, **kwargs)
        if args.reference and source == audio:
            summary['cer'] = cer(args.reference, summary['text'])
        summary['matchedHotwords'] = [word for word in args.hotword if word in summary['text']]
        report['cases'].append(summary)
        save()
        print(json.dumps(summary, ensure_ascii=False), flush=True)
        return result

    try:
        case('greedy', 'greedy_search')
        case('beam-unhinted', 'modified_beam_search')
        case('beam-hinted', 'modified_beam_search', '\n'.join(args.hotword))
        if args.comparison_only:
            report['passed'] = True
            return
        silence = output / 'silence.wav'
        write_audio(silence, bytes(3 * 16000 * 2))
        silent = case('silence-hinted', 'modified_beam_search', '\n'.join(args.hotword), silence)
        if silent['segments']:
            raise AssertionError('silence produced transcript with hints')
        frames, pcm = audio_info(audio)
        long_audio = output / 'long.wav'
        repeat = max(1, math.ceil(31 * 16000 / frames))
        write_audio(long_audio, pcm * repeat)
        case('long-input', 'greedy_search', source=long_audio)
        invalid = 'MON :99\nMON/OTHER\n<unk>'
        diagnostics = case('ignored-spellings', None, invalid)
        if set(diagnostics['ignoredHotwords']) != set(invalid.splitlines()) or set(diagnostics['ignoredHotwordReasons']) != set(invalid.splitlines()):
            raise AssertionError('ignored terms/reasons do not match rejected spellings')
        if args.legacy_worker:
            if not args.reazon_assets:
                parser.error('--legacy-worker requires --reazon-assets')
            reazon = args.reazon_assets.resolve(strict=True)
            legacy = args.legacy_worker.resolve(strict=True)
            for suffix, hints in [('plain', ''), ('hinted', '\u4f1a\u8b70\n\u78ba\u8a8d')]:
                before_summary, before = run_case(legacy, library, reazon, audio, output,
                                                  'reazon-old-' + suffix, None, hints, haas=False)
                after_summary, after = run_case(worker, library, reazon, audio, output,
                                                'reazon-new-' + suffix, None, hints, haas=False)
                if suffix == 'plain' and before != after:
                    raise AssertionError('legacy unhinted Reazon output changed')
                if suffix == 'hinted' and after['ignoredHotwords']:
                    raise AssertionError('valid tab-delimited Reazon terms were ignored')
                report['cases'].extend([before_summary, after_summary])
                save()
            report['legacyReazonPlainOutputEqual'] = True
            report['reazonTabDelimitedHintsApplied'] = True
        report['passed'] = True
    except BaseException as error:
        report['passed'] = False
        report['error'] = str(error)
        raise
    finally:
        save()


if __name__ == '__main__':
    main()
