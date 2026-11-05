"""Opt-in offline native regression/benchmark. Use synthetic speech only.

MON_RESIDENT_RUNTIME: worker package directory
MON_RESIDENT_INPUT_WAV: synthetic mono PCM16 16k speech WAV
MON_RESIDENT_WHISPER_MODEL / MON_RESIDENT_WHISPER_CLI
MON_RESIDENT_LLAMA_MODEL / MON_RESIDENT_LLAMA_CLI (Qwen 3 regression)
MON_RESIDENT_SHERPA_ROOT: package with bin/, models/, lib/
MON_RESIDENT_EMBEDDING_HELPER: existing single-shot MON helper
MON_RESIDENT_VAD_MODEL: optional pinned Silero VAD model
MON_RESIDENT_METAL: opt into actual Metal GPU tests
MON_RESIDENT_VULKAN: opt into the Windows Vulkan memory-estimator test
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


@unittest.skipUnless(os.environ.get('MON_RESIDENT_RUNTIME') and os.environ.get('MON_RESIDENT_LLAMA_MODEL'),
                     'explicit native llama worker and model required')
class NativeLlamaMemory(unittest.TestCase):
    """Runs on Windows and macOS without audio or platform-specific timing tools."""

    def run_requests(self, operations, vulkan=False):
        temporary = tempfile.TemporaryDirectory(prefix='mon-llama-memory-check-')
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name).resolve()
        runtime = Path(os.environ['MON_RESIDENT_RUNTIME']).resolve()
        executable = runtime / ('mon-llama-vulkan-worker.exe' if vulkan else
                                'mon-llama-worker.exe' if os.name == 'nt' else 'mon-llama-worker')
        requests = [dict(protocol=1, id=1, op='init')]
        directories = []
        for index, (kind, context) in enumerate(operations, 1):
            directory = root / str(index)
            directory.mkdir()
            directories.append(directory)
            (directory / 'system.txt').write_text('Return only the JSON object required by the schema.', encoding='utf8')
            (directory / 'prompt.txt').write_text('Return {"ok":true}.', encoding='utf8')
            (directory / 'schema.json').write_text(json.dumps(dict(type='object', properties=dict(ok=dict(type='boolean', const=True)),
                                                                  required=['ok'], additionalProperties=False)), encoding='utf8')
            for name in ('result.json', 'stderr.log'):
                (directory / name).touch()
            args = ['--model', os.environ['MON_RESIDENT_LLAMA_MODEL'], '--ctx-size', str(context),
                    '--predict', '64', '--threads', '2', '--threads-batch', '2', '--gpu-layers', '0',
                    '--device', 'none', '--no-kv-offload', '--no-op-offload', '--offline', '--fit', 'off',
                    '--system-prompt-file', str(directory / 'system.txt'), '--file', str(directory / 'prompt.txt'),
                    '--json-schema-file', str(directory / 'schema.json'), '--conversation', '--jinja',
                    '--no-escape', '--reasoning', 'off', '--seed', '42', '--temp', '0', '--single-turn',
                    '--no-display-prompt', '--no-context-shift', '--no-warmup', '--simple-io']
            if vulkan:
                args[args.index('--gpu-layers') + 1] = '16'
                args[args.index('--device') + 1] = 'Vulkan0'
                args.remove('--no-kv-offload')
                args.remove('--no-op-offload')
            requests.append(dict(protocol=1, id=index, op='infer', kind=kind, args=args,
                                 directory=str(directory), stdout='result.json', stderr='stderr.log'))
        requests.append(dict(protocol=1, id=0, op='shutdown'))
        completed = subprocess.run([str(executable), '--mon-resident-v1'],
                                   input=''.join(json.dumps(request) + '\n' for request in requests).encode(),
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=180)
        diagnostics = completed.stderr.decode(errors='replace') + '\n'.join(
            (directory / 'stderr.log').read_text(encoding='utf8', errors='replace') for directory in directories)
        self.assertEqual(completed.returncode, 0, diagnostics[-8000:])
        replies = [json.loads(row) for row in completed.stdout.splitlines()]
        self.assertFalse(any(reply['op'] == 'error' for reply in replies), diagnostics[-8000:])
        self.assertEqual([reply['id'] for reply in replies if reply['op'] == 'done'], list(range(1, len(operations) + 1)))
        return directories, diagnostics

    def test_cpu_memory_estimate_repeats_without_loading_model_tensors(self):
        directories, diagnostics = self.run_requests([('llama_memory', 2048), ('llama_memory', 8192), ('llama_memory', 2048)])
        estimates = []
        for directory, context in zip(directories, (2048, 8192, 2048)):
            output = (directory / 'result.json').read_bytes()
            self.assertLessEqual(len(output), 32 * 1024)
            estimate = json.loads(output)
            estimates.append(estimate)
            self.assertEqual((estimate['schemaVersion'], estimate['contextTokens'], estimate['gpuLayers']), (1, context, 0))
            self.assertEqual(len(estimate['devices']), 1)
            host = estimate['devices'][0]
            self.assertEqual((host['type'], host['name'], host['sharedMemory']), ('host', 'CPU', False))
            self.assertTrue(all(isinstance(host[key], int) and host[key] >= 0 for key in
                                ('total', 'free', 'modelBytes', 'contextBytes', 'computeBytes')))
            self.assertLessEqual(host['free'], host['total'])
            self.assertGreater(host['modelBytes'], 0)
            self.assertGreater(host['contextBytes'], 0)
        for field in ('modelBytes', 'contextBytes', 'computeBytes'):
            self.assertEqual(estimates[0]['devices'][0][field], estimates[2]['devices'][0][field])
        self.assertGreaterEqual(estimates[1]['devices'][0]['contextBytes'], estimates[0]['devices'][0]['contextBytes'])
        self.assertNotIn('mon_model_load: llama=', diagnostics)
        self.assertNotIn('using device ', diagnostics)

    def test_memory_estimate_does_not_change_completion_resident_cache(self):
        directories, diagnostics = self.run_requests([('llama_memory', 8192), ('llama', 2048),
                                                      ('llama_memory', 8192), ('llama', 2048)])
        self.assertEqual(diagnostics.count('mon_model_load: llama=1'), 1)
        for directory in (directories[1], directories[3]):
            raw = (directory / 'result.json').read_text(encoding='utf8').lstrip()
            self.assertEqual(json.JSONDecoder().raw_decode(raw)[0], {'ok': True})

    def test_large_context_is_estimated_without_initializing_completion(self):
        directories, diagnostics = self.run_requests([('llama_memory', 131072)])
        estimate = json.loads((directories[0] / 'result.json').read_bytes())
        self.assertEqual((estimate['schemaVersion'], estimate['contextTokens'], estimate['gpuLayers']),
                         (1, 131072, 0))
        self.assertEqual(len(estimate['devices']), 1)
        self.assertGreater(estimate['devices'][0]['contextBytes'], 0)
        self.assertNotIn('mon_model_load: llama=', diagnostics)
        REPORT.append(dict(test='cpu-memory-131072', estimate=estimate))

    @unittest.skipUnless(os.name == 'nt' and os.environ.get('MON_RESIDENT_VULKAN'),
                         'explicit Windows Vulkan memory estimate required')
    def test_vulkan_estimate_identifies_device_and_shared_host_demand(self):
        directories, diagnostics = self.run_requests([('llama_memory', 8192)], vulkan=True)
        estimate = json.loads((directories[0] / 'result.json').read_bytes())
        self.assertEqual((estimate['schemaVersion'], estimate['contextTokens']), (1, 8192))
        self.assertGreater(estimate['gpuLayers'], 0)
        self.assertGreaterEqual(len(estimate['devices']), 2)
        self.assertEqual(estimate['devices'][-1]['type'], 'host')
        gpu = estimate['devices'][0]
        self.assertEqual(gpu['name'], 'Vulkan0')
        self.assertIn(gpu['type'], ('gpu', 'integratedGpu'))
        if gpu['type'] == 'integratedGpu':
            self.assertTrue(gpu['sharedMemory'])
        self.assertGreater(gpu['modelBytes'] + gpu['contextBytes'] + gpu['computeBytes'], 0)
        self.assertNotIn('mon_model_load: llama=', diagnostics)
        REPORT.append(dict(test='vulkan-memory-8192', estimate=estimate))


class Worker:
    def __init__(self, test, engine):
        self.test, self.engine = test, engine
        self.log = test.root / (engine + '-startup.log')
        self.stderr = self.log.open('w')
        self.proc = subprocess.Popen(['/usr/bin/time', '-l', str(test.runtime / ('mon-' + engine + '-worker')), '--mon-resident-v1'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr, env=getattr(test, 'worker_env', None))
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
                raise AssertionError('resident worker died: ' + self.log.read_text(errors='replace')[-2000:])
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
                self.test.fail(str(response) + '\n' + (directory / 'stderr.log').read_text(errors='replace')[-4000:])
            if response['op'] == 'done':
                break
        self.inference_seconds += time.monotonic()-started
        pid = re.search(r'mon_session: pid=(\d+)', (directory/'stderr.log').read_text(errors='replace'))[1]
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
            if kind == 'llama_prefix':
                self.assertEqual((resident/output).read_bytes(), (old/output).read_bytes(), 'generated output bytes changed under prefix reuse')
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
            def args(d, i):
                values = args(d, i)
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
        self.check_sequence('llama',os.environ['MON_RESIDENT_LLAMA_CLI'],args,lambda d:json.JSONDecoder().raw_decode((d/'result.json').read_text().lstrip())[0],output='result.json',kind=getattr(self,'llama_kind',None))
        if getattr(self, 'qwen3', False):
            REPORT[-1]['engine'] = 'llama_qwen3_sampling_reuse'
        if getattr(self, 'metal', False):
            for i in range(1,4):
                text = (self.root/('resident-'+str(i))/'stderr.log').read_text()
                self.assertRegex(text,r'using device MTL0')
                self.assertRegex(text,r'offloaded [1-9][0-9]*/[1-9][0-9]* layers to GPU')
            REPORT[-1]['engine'] = 'llama_qwen3_metal_trial_reuse' if getattr(self, 'qwen3', False) else 'llama_metal_trial_reuse'

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_LLAMA_MODEL'), 'llama model not provided')
    def test_llama_prefix_reuses_only_system_and_preserves_json(self):
        self.llama_kind = 'llama_prefix'
        self.test_llama_schema_seed_and_context_reset()
        hits = [int(re.search(r'mon_prefix_cache: tokens=(\d+)', (self.root / ('resident-' + str(i)) / 'stderr.log').read_text())[1]) for i in range(1, 4)]
        self.assertEqual(hits[0], 0)
        self.assertGreater(hits[1], 0)
        self.assertEqual(hits[1], hits[2])
        REPORT[-1]['reusedSystemTokens'] = hits

    @unittest.skipUnless(os.environ.get('MON_RESIDENT_LLAMA_MODEL'), 'llama model not provided')
    def test_llama_tokenizer_matches_completion_prompt_tokens(self):
        worker = Worker(self, 'llama')
        d = self.directory('token-count', self.speech)
        (d / 'system.txt').write_text('Return the requested JSON. 日本語の指示文。')
        (d / 'prompt.txt').write_text('Return {"ok":true}')
        (d / 'schema.json').write_text('{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}')
        (d / 'count.json').touch()
        args = ['--model',os.environ['MON_RESIDENT_LLAMA_MODEL'],'--system-prompt-file',str(d/'system.txt'),'--file',str(d/'prompt.txt'),'--conversation','--jinja','--no-escape','--reasoning','off','--offline']
        worker.run(1,args,d,'count.json',kind='llama_tokenize')
        count = json.loads((d/'count.json').read_text())
        self.assertGreater(count['tokens'], count['systemPrefixTokens'])
        self.assertGreater(count['systemPrefixTokens'], 0)
        args += ['--json-schema-file',str(d/'schema.json'),'--ctx-size','2048','--predict','64','--threads','2','--threads-batch','2','--gpu-layers','0','--device','none','--no-kv-offload','--no-op-offload','--seed','42','--temp','0','--single-turn','--no-display-prompt','--no-context-shift','--no-warmup','--fit','off','--simple-io','--log-verbosity','4','--verbose-prompt']
        worker.run(2,args,d,'result.json',kind='llama')
        text=(d/'stderr.log').read_text()
        measured = re.search(r'number of tokens in prompt = (\d+)',text)
        self.assertIsNotNone(measured)
        self.assertEqual(count['tokens'], int(measured[1]))
        self.assertEqual(json.JSONDecoder().raw_decode((d/'result.json').read_text().lstrip())[0], {'ok':True})
        worker.close()

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
