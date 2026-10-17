"""Opt-in offline native regression/benchmark. Use synthetic speech only.

MON_RESIDENT_RUNTIME: worker package directory
MON_RESIDENT_INPUT_WAV: synthetic mono PCM16 16k speech WAV
MON_RESIDENT_WHISPER_MODEL / MON_RESIDENT_WHISPER_CLI
MON_RESIDENT_LLAMA_MODEL / MON_RESIDENT_LLAMA_CLI (Qwen2.5 regression)
MON_RESIDENT_SHERPA_ROOT: package with bin/, models/, lib/
MON_RESIDENT_EMBEDDING_HELPER: existing single-shot MON helper
MON_RESIDENT_VAD_MODEL: optional pinned Silero VAD model
MON_RESIDENT_METAL: opt into actual Metal GPU tests
MON_RESIDENT_LLAMA_QWEN3_MODEL: optional Qwen3 model for template/sampling tests
MON_RESIDENT_REPORT: optional JSON measurements destination
"""
import json
import os
from pathlib import Path
import re
import select
import shutil
import subprocess
import tempfile
import time
import unittest
import wave

REPORT = []


class Worker:
    def __init__(self, test, engine):
        self.test, self.engine = test, engine
        self.log = test.root / (engine + '-startup.log')
        self.stderr = self.log.open('w')
        self.proc = subprocess.Popen(['/usr/bin/time', '-l', str(test.runtime / ('mon-' + engine + '-worker')), '--mon-resident-v1'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr)
        self.pending = b''
        self.inference_seconds = 0.0
        self.rss = []
        test.addCleanup(self.cleanup)
        self.started = time.monotonic()
        self.send(dict(protocol=1, id=1, op='init'))
        assert self.receive()['op'] == 'ready'
        self.initialization_seconds = time.monotonic()-self.started

    def send(self, value):
        self.proc.stdin.write((json.dumps(value) + '\n').encode())
        self.proc.stdin.flush()

    def receive(self):
        while b'\n' not in self.pending:
            if not select.select([self.proc.stdout], [], [], 180)[0]:
                raise AssertionError('resident response timed out')
            block = os.read(self.proc.stdout.fileno(), 65536)
            if not block:
                raise AssertionError('resident worker died: ' + self.log.read_text()[-2000:])
            self.pending += block
        line, self.pending = self.pending.split(b'\n', 1)
        return json.loads(line)

    def run(self, index, args, directory, output, kind=None):
        started = time.monotonic()
        self.send(dict(protocol=1, id=index, op='infer', kind=kind or self.engine, args=args, directory=str(directory), stdout=output, stderr='stderr.log'))
        while True:
            response = self.receive()
            self.test.assertEqual(response['id'], index)
            if response['op'] == 'error':
                self.test.fail(str(response) + '\n' + (directory / 'stderr.log').read_text()[-4000:])
            if response['op'] == 'done':
                break
        self.inference_seconds += time.monotonic()-started
        pid = re.search(r'mon_session: pid=(\d+)', (directory/'stderr.log').read_text())[1]
        sample = subprocess.run(['/bin/ps','-o','rss=','-p',pid],capture_output=True,text=True)
        self.rss.append(int(sample.stdout.strip())*1024 if sample.returncode==0 and sample.stdout.strip() else None)

    def cleanup(self):
        if self.proc.poll() is None:
            # The subprocess is ours. Closing the input lets the worker exit
            # even if a result assertion failed before normal shutdown.
            self.proc.stdin.close()
            try: self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill(); self.proc.wait(timeout=10)
        for stream in (self.proc.stdin,self.proc.stdout,self.stderr):
            if not stream.closed: stream.close()

    def close(self):
        closing = time.monotonic()
        if self.proc.poll() is None:
            self.send(dict(protocol=1, id=0, op='shutdown'))
            self.proc.wait(timeout=15)
        self.stderr.close()
        self.proc.stdin.close()
        self.proc.stdout.close()
        self.test.assertEqual(self.proc.returncode, 0)
        match = re.search(r'(\d+)\s+maximum resident set size', self.log.read_text())
        return dict(elapsedSeconds=self.initialization_seconds+self.inference_seconds+time.monotonic()-closing, peakRssBytes=int(match[1]) if match else None, rssAfterUnitsBytes=self.rss)


@unittest.skipUnless(os.environ.get('MON_RESIDENT_RUNTIME') and os.environ.get('MON_RESIDENT_INPUT_WAV'), 'explicit native workers, models and synthetic speech required')
class NativeResident(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='mon-resident-native-check-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.runtime = Path(os.environ['MON_RESIDENT_RUNTIME']).resolve()
        self.speech = Path(os.environ['MON_RESIDENT_INPUT_WAV']).resolve()
        with wave.open(str(self.speech)) as source:
            self.frames = source.getnframes()
            self.assertEqual((source.getnchannels(), source.getsampwidth(), source.getframerate()), (1, 2, 16000))
            pcm = source.readframes(self.frames)
        self.speech = self.root / 'speech.wav'
        with wave.open(str(self.speech), 'w') as target:
            target.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
            target.writeframes(pcm)
        self.silence = self.root / 'silence.wav'
        with wave.open(str(self.silence), 'w') as target:
            target.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
            target.writeframes(b'\0\0' * self.frames)

    def directory(self, name, audio):
        directory = self.root / name
        directory.mkdir()
        shutil.copyfile(audio, directory / 'input.wav')
        for file in ('stdout.log', 'stderr.log', 'result.json'):
            (directory / file).touch()
        return directory

    def legacy(self, executable, args, directory, output):
        started = time.monotonic()
        with (directory / output).open('w') as stdout, (directory / 'stderr.log').open('w') as stderr:
            completed = subprocess.run(['/usr/bin/time', '-l', str(executable), *args], cwd=directory, stdout=stdout, stderr=stderr, timeout=180)
        self.assertEqual(completed.returncode, 0, (directory / 'stderr.log').read_text()[-3000:])
        text = (directory / 'stderr.log').read_text()
        match = re.search(r'(\d+)\s+maximum resident set size', text)
        return dict(elapsedSeconds=time.monotonic()-started, peakRssBytes=int(match[1]) if match else None)

    def check_sequence(self, engine, executable, arguments, read, output='stdout.log', kind=None, legacy_args=None):
        worker = Worker(self, engine)
        native, baseline, measurements, pids = [], [], [], []
        for index, audio in enumerate((self.speech, self.silence, self.speech), 1):
            resident = self.directory('resident-' + str(index), audio)
            old = self.directory('legacy-' + str(index), audio)
            args = arguments(resident, index)
            worker.run(index, args, resident, output, kind)
            pids += re.findall(r'mon_session: pid=(\d+)', (resident / 'stderr.log').read_text())
            measurements.append(self.legacy(executable, (legacy_args or arguments)(old, index), old, output))
            native.append(read(resident))
            baseline.append(read(old))
        metrics = worker.close()
        self.assertEqual(len(set(pids)), 1)
        self.assertEqual(native, baseline)
        self.assertEqual(native[0], native[2], 'prior unit leaked into inference state')
        load_lines = sum((self.root / ('resident-' + str(i)) / 'stderr.log').read_text().count('mon_model_load:') for i in range(1, 4))
        self.assertEqual(load_lines, 1)
        if all(v is not None for v in metrics['rssAfterUnitsBytes']):
            self.assertLessEqual(metrics['rssAfterUnitsBytes'][-1], metrics['rssAfterUnitsBytes'][0] + 128*1024*1024, 'resident memory grows across requests')
        REPORT.append(dict(engine=kind or engine, resident=metrics, legacy=measurements, residentModelLoads=load_lines, legacyModelLoads=3, outputsMatch=True, pidReused=True))

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_WHISPER_MODEL'), 'Whisper model not provided')
    def test_whisper_speech_silence_speech(self):
        model = os.environ['MON_RESIDENT_WHISPER_MODEL']
        def args(d, i):
            return ['--model', model, '--file', str(d/'input.wav'), '--language', 'ja', '--output-json', '--output-file', str(d/'result'), '--threads', '2', '--beam-size', '5', '--no-gpu', '--suppress-nst']
        self.check_sequence('whisper', os.environ['MON_RESIDENT_WHISPER_CLI'], args, lambda d: json.loads((d/'result.json').read_text())['transcription'])

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_VAD_MODEL'), 'VAD model not provided')
    def test_whisper_vad_model_and_recurrent_state_reuse(self):
        def args(d, i):
            return ['--model',os.environ['MON_RESIDENT_WHISPER_MODEL'],'--file',str(d/'input.wav'),'--language','ja','--output-json','--output-file',str(d/'result'),'--threads','2','--beam-size','5','--no-gpu','--suppress-nst','--vad','--vad-model',os.environ['MON_RESIDENT_VAD_MODEL'],'--vad-threshold','0.60','--vad-min-speech-duration-ms','300','--vad-min-silence-duration-ms','100','--vad-max-speech-duration-s','30','--vad-speech-pad-ms','30','--vad-samples-overlap','0.10']
        self.check_sequence('whisper',os.environ['MON_RESIDENT_WHISPER_CLI'],args,lambda d:json.loads((d/'result.json').read_text())['transcription'])
        logs = [(self.root/('resident-'+str(i))/'stderr.log').read_text() for i in range(1,4)]
        self.assertEqual(sum(text.count('loading VAD model') for text in logs),1)
        REPORT[-1]['engine'] = 'whisper_vad'

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_LLAMA_MODEL'), 'llama model not provided')
    def test_llama_schema_seed_and_context_reset(self):
        def args(d, i):
            (d/'system.txt').write_text('Return exactly the JSON object requested by the user.')
            (d/'prompt.txt').write_text('Return {"ok":' + ('false' if i == 2 else 'true') + '}')
            (d/'schema.json').write_text(json.dumps(dict(type='object',properties=dict(ok=dict(type='boolean')),required=['ok'],additionalProperties=False)))
            return ['--model',os.environ['MON_RESIDENT_LLAMA_MODEL'],'--file',str(d/'prompt.txt'),'--system-prompt-file',str(d/'system.txt'),'--json-schema-file',str(d/'schema.json'),'--threads','2','--threads-batch','2','--ctx-size','2048','--predict','64','--gpu-layers','0','--seed','42','--temp','0','--conversation','--jinja','--single-turn','--no-display-prompt','--no-context-shift','--no-escape','--offline','--no-warmup','--fit','off','--reasoning','off','--color','off','--log-colors','off','--simple-io','--device','none','--no-kv-offload','--no-op-offload']
        if getattr(self, 'qwen3', False):
            qwen25_args = args
            def args(d, i):
                values = qwen25_args(d, i)
                values[values.index('--model')+1] = os.environ['MON_RESIDENT_LLAMA_QWEN3_MODEL']
                values[values.index('--temp')+1] = '0.7'
                template = r"""{%- for message in messages %}{{- '<|im_start|>' + message.role + '\n' + message.content + '<|im_end|>\n' }}{%- endfor %}{%- if add_generation_prompt %}{{- '<|im_start|>assistant\n<think>\n\n</think>\n\n' }}{%- endif %}"""
                return values + ['--top-p','0.8','--top-k','20','--min-p','0','--presence-penalty','1.5','--chat-template',template]
        if getattr(self, 'metal', False):
            cpu_args = args
            def args(d, i):
                values = cpu_args(d, i)
                # AutoMetalV2 probes an upper bound, then records the actual
                # offloaded count for subsequent extraction/merge requests.
                layers = '256'
                if i > 1:
                    trial_log = (self.root/'resident-1'/'stderr.log').read_text()
                    layers = re.search(r'offloaded (\d+)/\d+ layers to GPU', trial_log)[1]
                    self.assertLess(int(layers), 256)
                values[values.index('--gpu-layers')+1] = layers
                values[values.index('--device')+1] = 'MTL0'
                values.remove('--no-kv-offload'); values.remove('--no-op-offload')
                values += ['--log-verbosity','4']
                return values
        self.check_sequence('llama',os.environ['MON_RESIDENT_LLAMA_CLI'],args,lambda d:json.JSONDecoder().raw_decode((d/'result.json').read_text().lstrip())[0],output='result.json')
        if getattr(self, 'qwen3', False):
            REPORT[-1]['engine'] = 'llama_qwen3_sampling_reuse'
        if getattr(self, 'metal', False):
            for i in range(1,4):
                text = (self.root/('resident-'+str(i))/'stderr.log').read_text()
                self.assertRegex(text,r'using device MTL0')
                self.assertRegex(text,r'offloaded [1-9][0-9]*/[1-9][0-9]* layers to GPU')
            REPORT[-1]['engine'] = 'llama_qwen3_metal_trial_reuse' if getattr(self, 'qwen3', False) else 'llama_metal_trial_reuse'

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_LLAMA_QWEN3_MODEL'), 'explicit Qwen3 model required')
    def test_llama_qwen3_sampling_template_and_seed_reset(self):
        self.qwen3 = True
        self.test_llama_schema_seed_and_context_reset()

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_METAL'), 'explicit Metal test required')
    def test_llama_metal_trial_keeps_model_for_following_schema(self):
        self.metal = True
        self.test_llama_schema_seed_and_context_reset()

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_METAL') and os.environ.get('MON_RESIDENT_LLAMA_QWEN3_MODEL'), 'explicit Metal and Qwen3 model required')
    def test_llama_qwen3_metal_trial_keeps_model_for_following_schema(self):
        self.qwen3 = True
        self.metal = True
        self.test_llama_schema_seed_and_context_reset()


    @unittest.skipUnless(os.environ.get('MON_RESIDENT_SHERPA_ROOT'), 'Sherpa package not provided')
    def test_sherpa_track_settings_reset(self):
        package = Path(os.environ['MON_RESIDENT_SHERPA_ROOT'])
        def legacy(d, i):
            return [f'--segmentation.pyannote-model={package}/models/segmentation.onnx',f'--embedding.model={package}/models/embedding.onnx','--segmentation.num-threads=2','--embedding.num-threads=2','--segmentation.pyannote-window-shift-ratio=0.5','--clustering.num-clusters=' + ('2' if i==2 else '-1'),'--clustering.cluster-threshold=0.5','--clustering.compute-confidence=false','--min-duration-on=0.3','--min-duration-off=0.5',str(d/'input.wav')]
        def resident(d, i): return legacy(d, i) + ['--library',str(package/'lib/libsherpa-onnx-c-api.dylib')]
        self.check_sequence('sherpa',package/'bin/sherpa-onnx-offline-speaker-diarization',resident,lambda d:(d/'stdout.log').read_text().strip(),legacy_args=legacy)

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_EMBEDDING_HELPER'), 'embedding helper not provided')
    def test_embedding_windows_reuse_extractor(self):
        package = Path(os.environ['MON_RESIDENT_SHERPA_ROOT'])
        def args(d,i):
            return ['--protocol-version','1','--library',str(package/'lib/libsherpa-onnx-c-api.dylib'),'--model',str(package/'models/embedding.onnx'),'--wave',str(d/'input.wav'),'--first-frame','0','--end-frame',str(min(self.frames,480000)),'--threads','2']
        self.check_sequence('sherpa',os.environ['MON_RESIDENT_EMBEDDING_HELPER'],args,lambda d:json.loads((d/'stdout.log').read_text()),kind='embedding')


def tearDownModule():
    if REPORT and os.environ.get('MON_RESIDENT_REPORT'):
        Path(os.environ['MON_RESIDENT_REPORT']).write_text(json.dumps(REPORT, indent=2)+'\n')
