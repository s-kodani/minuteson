import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import mon_model_catalog as catalog
import mon_windows_runtime as windows_runtime
import mon_asr_assets as asr_assets


def canonical_hash(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True,
                                     separators=(',', ':')).encode()).hexdigest()


class ModelCatalogTests(unittest.TestCase):
    def test_windows_packages_bind_all_llm_choices_to_the_fixed_runtime(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / 'package'
            executables = {'whisper': 'whisper-cli.exe', 'sherpa': 'sherpa-onnx-offline-speaker-diarization.exe',
                           'llama': 'llama-completion.exe', 'llama11960': 'llama-completion.exe'}
            for engine, name in executables.items():
                (package / engine).mkdir(parents=True)
                (package / engine / name).write_bytes(engine.encode())
                if engine.startswith('llama'):
                    (package / engine / 'llama.dll').write_bytes((engine + '-library').encode())
            redist = root / 'Microsoft Visual Studio/2022/BuildTools/VC/Redist/MSVC/14.0/x64/Microsoft.VC143.CRT'
            redist.mkdir(parents=True)
            for name in ('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll'):
                (redist / name).write_bytes(name.encode())
            with patch.object(windows_runtime, 'PACKAGE', package), patch.object(windows_runtime, 'sign'), \
                    patch.object(windows_runtime, 'visual_cpp_redistributable', return_value=redist):
                windows_runtime.catalog()
            choices = json.loads((package / 'catalog.json').read_text(encoding='utf8'))
            llms = [c for c in choices if c['recipe']['manifest']['engine'] == 'llama.cpp']
            self.assertEqual(len(llms), 9)
            for choice in llms:
                manifest = choice['recipe']['manifest']
                self.assertEqual(manifest['engineVersion'], 'b11960')
                engine = 'llama11960'
                executable = next(f for f in manifest['files'] if f['role'] == 'executable')
                self.assertEqual(executable['expectedHash'], hashlib.sha256(engine.encode()).hexdigest())
                source = next(s for s in choice['recipe']['sources'] if s['relativePath'] == executable['relativePath'])
                self.assertEqual(source['archive']['memberPath'], engine + '/llama-completion.exe')
                self.assertTrue(choice['id'].startswith('windows-llama-'))

    def test_generated_macos_recipes_bind_the_reviewed_runtime_and_preserve_asr(self):
        expected = {
            'macos.json': ('base', '0c1632dca27daa2b0174c4b5a1e94c33e2075717053397458988751260cdd902'),
            'asr-transducer.json': ('asr', 'd72e39de3b7a96da478e4d267bf88f83400dee5fc64cb35d4deaade73eadc8a1'),
        }
        for name, (group, golden) in expected.items():
            generated = catalog.compile_target('macos_aarch64', group)
            self.assertEqual(generated, json.loads((catalog.CATALOG / name).read_text(encoding='utf8')))
            legacy = generated if group == 'base' else [choice for choice in generated if choice['id'] in (
                'reazonspeech-k2-v2-int8-fp32', 'reazonspeech-k2-v2-fp32')]
            self.assertEqual(canonical_hash(legacy), golden)

    def test_windows_model_templates_match_previous_recipe_transform_without_mac_source(self):
        baseline = json.loads((catalog.CATALOG / 'macos.json').read_text(encoding='utf8'))
        expected = []
        for original in baseline:
            choice = copy.deepcopy(original)
            choice['id'] = 'windows-' + choice['id']
            engine = choice['recipe']['manifest']['engine']
            choice['label'] += '（Windows CPU）' if engine == 'sherpa-onnx' else '（Windows CPU／Vulkan）'
            manifest = choice['recipe']['manifest']
            manifest['target'] = 'windows_x86_64'
            pairs = [(f, s) for f, s in zip(manifest['files'], choice['recipe']['sources'])
                     if f['role'] not in ('executable', 'runtime_library')]
            manifest['files'] = [f for f, _ in pairs]
            choice['recipe']['sources'] = [s for _, s in pairs]
            expected.append(choice)
        self.assertEqual(catalog.compile_target('windows_x86_64'), expected)

    def test_asr_windows_templates_preserve_neutral_weights_and_explicit_backend_binding(self):
        mac = catalog.compile_target('macos_aarch64', 'asr')
        windows = catalog.compile_target('windows_x86_64', 'asr')
        for source, target in zip(mac, windows):
            self.assertEqual(target['id'], 'windows-' + source['id'])
            manifest = target['recipe']['manifest']
            self.assertEqual(manifest['backend']['files']['library'], 'lib/sherpa-onnx-c-api.dll')
            self.assertEqual(manifest['files'], [f for f in source['recipe']['manifest']['files']
                                               if f['role'] != 'runtime_library'])

    def test_platform_only_model_does_not_need_a_macos_definition(self):
        definitions = catalog.definitions()
        addition = copy.deepcopy(definitions[0])
        addition['id'] = 'windows-only-llm'
        addition['targets'] = ['windows_x86_64']
        with patch.object(catalog, 'definitions', return_value=definitions + [addition]):
            self.assertIn('windows-windows-only-llm', [c['id'] for c in catalog.compile_target('windows_x86_64')])
            self.assertNotIn('windows-only-llm', [c['id'] for c in catalog.compile_target('macos_aarch64')])

    def test_haas_external_data_and_hotword_assets_are_pinned_for_both_targets(self):
        for target in ('macos_aarch64', 'windows_x86_64'):
            choice = next(c for c in catalog.compile_target(target, 'asr')
                          if c['id'].endswith('haas-ja-v1.0-onnx-quantized'))
            manifest = choice['recipe']['manifest']
            backend = manifest['backend']
            self.assertEqual(backend['id'], 'sherpa-onnx-nemo-tdt')
            self.assertEqual(set(backend['files']), {'bpeVocab', 'data', 'decoder', 'joiner', 'library', 'tokens'})
            self.assertEqual(backend['files']['data'], 'models/encoder.data')
            self.assertEqual(backend['files']['bpeVocab'], 'models/bpe.vocab')
            primary = next(f for f in manifest['files'] if f['role'] == 'asr')
            self.assertEqual(primary['relativePath'], 'models/encoder.quantized.onnx')
            self.assertEqual(Path(primary['relativePath']).parent, Path(backend['files']['data']).parent)
            for file, source in zip(manifest['files'], choice['recipe']['sources']):
                if file['role'] in ('asr', 'asr_asset') and file['relativePath'] != 'models/bpe.vocab':
                    self.assertIn('/resolve/7f5ab0a4b1bf39db78afd6691eeecbc4f9d42e39/', source['url'])
            self.assertIn('notices/haas-NOTICE.txt', {f['relativePath'] for f in manifest['files']})

    def test_haas_hotword_asset_reproduces_offline_from_pinned_tokens(self):
        for relative, expected in asr_assets.outputs().items():
            self.assertEqual((asr_assets.DIRECTORY / relative).read_bytes(), expected)
        vocabulary = (asr_assets.DIRECTORY / 'models/bpe.vocab').read_text(encoding='utf8')
        self.assertEqual(len(vocabulary.splitlines()), 3073)
        self.assertTrue(vocabulary.startswith('<unk>\t-1.0\n'))
        self.assertTrue(vocabulary.endswith('<blk>\t-1.0\n'))

    def test_duplicate_definitions_and_unknown_target_fail(self):
        definitions = catalog.definitions()
        with patch.object(catalog, 'definitions', return_value=definitions + [definitions[0]]):
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                catalog.compile_target('macos_aarch64')
        with self.assertRaisesRegex(ValueError, 'unsupported'):
            catalog.compile_target('unknown')


if __name__ == '__main__':
    unittest.main()
