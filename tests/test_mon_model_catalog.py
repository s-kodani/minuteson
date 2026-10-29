import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import mon_model_catalog as catalog


def canonical_hash(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True,
                                     separators=(',', ':')).encode()).hexdigest()


class ModelCatalogTests(unittest.TestCase):
    def test_generated_macos_compatibility_recipes_retain_every_old_field_and_order(self):
        expected = {
            'macos.json': ('base', 'f65b4e440155913c85c8ab67b4fe1554635a9873301f3922954037759b2d8445'),
            'asr-transducer.json': ('asr', 'd72e39de3b7a96da478e4d267bf88f83400dee5fc64cb35d4deaade73eadc8a1'),
        }
        for name, (group, golden) in expected.items():
            generated = catalog.compile_target('macos_aarch64', group)
            self.assertEqual(generated, json.loads((catalog.CATALOG / name).read_text()))
            self.assertEqual(canonical_hash(generated), golden)

    def test_windows_model_templates_match_previous_recipe_transform_without_mac_source(self):
        baseline = json.loads((catalog.CATALOG / 'macos.json').read_text())
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

    def test_duplicate_definitions_and_unknown_target_fail(self):
        definitions = catalog.definitions()
        with patch.object(catalog, 'definitions', return_value=definitions + [definitions[0]]):
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                catalog.compile_target('macos_aarch64')
        with self.assertRaisesRegex(ValueError, 'unsupported'):
            catalog.compile_target('unknown')


if __name__ == '__main__':
    unittest.main()
