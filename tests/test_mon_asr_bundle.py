import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('mon_asr_bundle', Path(__file__).resolve().parents[1]/'tools/mon_asr_bundle.py')
bundle = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bundle)

class AsrBundleTests(unittest.TestCase):
    def test_runtime_archive_is_deterministic_and_binds_signed_worker_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            (root/'mon-transducer-worker').write_bytes(b'signed native worker')
            (root/'nlohmann-MIT.txt').write_bytes(b'MIT notice')
            first=bundle.assemble(root)
            self.assertEqual(first,bundle.assemble(root))
            with tarfile.open(root/'asr-runtime.tar') as archive:
                self.assertEqual(archive.getnames()[:2],['bin/mon-transducer-worker','notices/nlohmann-MIT.txt'])
                members = bundle.asset_members()
                self.assertEqual(archive.getnames()[2:], [member for _, _, _, member in members])
                for _, file, _, member in members:
                    self.assertEqual(len(archive.extractfile(member).read()), file['expectedSize'])
            self.assertEqual(len(first['files']), 2)
            self.assertEqual(set(first['modelAssets']), {'haas-ja-v1.0-onnx-quantized'})
            self.assertEqual(first['modelAssets']['haas-ja-v1.0-onnx-quantized'][0]['role'], 'asr_asset')
            (root/'mon-transducer-worker').write_bytes(b'resigned native worker')
            second=bundle.assemble(root)
            self.assertNotEqual(first['archiveSha256'],second['archiveSha256'])
            self.assertNotEqual(first['files'][0]['expectedHash'],second['files'][0]['expectedHash'])

    def test_windows_catalog_uses_packaged_dlls_and_preserves_fixed_model_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            for path,data in [('workers/mon-transducer-worker.exe',b'Windows worker'),('workers/nlohmann-MIT.txt',b'MIT'),('sherpa/sherpa-onnx-c-api.dll',b'C API'),('sherpa/onnxruntime.dll',b'ORT')]:
                file=root/path;file.parent.mkdir(parents=True,exist_ok=True);file.write_bytes(data)
            with tarfile.open(root/'runtime.tar','w') as archive:
                entry=tarfile.TarInfo('old/runtime.dll');entry.size=3;archive.addfile(entry,io.BytesIO(b'old'))
            old_source=dict(relativePath='runtime.dll',url='mon-bundled://windows-runtime-v1/runtime.tar',archive=dict(format='tar',expectedSize=1,expectedHash='0'*64,memberPath='old/runtime.dll'))
            (root/'catalog.json').write_text(json.dumps([dict(recipe=dict(manifest=dict(engine='whisper.cpp'),sources=[old_source]))]))
            bundle.append_windows_choices(root)
            choices=json.loads((root/'catalog.json').read_text(encoding='utf8'))
            self.assertEqual(len(choices),4)
            expected=bundle.digest(root/'runtime.tar')
            for choice in choices:
                for source in choice['recipe']['sources']:
                    if source['url']==old_source['url']:self.assertEqual(source['archive']['expectedHash'],expected)
            for choice in choices[1:]:
                manifest=choice['recipe']['manifest']
                self.assertEqual(manifest['target'],'windows_x86_64')
                self.assertEqual(manifest['backend']['files']['library'],'lib/sherpa-onnx-c-api.dll')
                self.assertEqual(len([f for f in manifest['files'] if f['role']=='executable']),1)
                self.assertEqual(len([f for f in manifest['files'] if f['role']=='runtime_library']),2)
                self.assertTrue(all(f['relativePath'].endswith('.dll') for f in manifest['files'] if f['role']=='runtime_library'))
                if manifest['backend']['id'] == 'sherpa-onnx-nemo-tdt':
                    self.assertEqual(manifest['modelVersion'],'7f5ab0a4b1bf39db78afd6691eeecbc4f9d42e39')
                    asset = next(f for f in manifest['files'] if f['relativePath'] == 'models/bpe.vocab')
                    source = next(s for s in choice['recipe']['sources'] if s['relativePath'] == asset['relativePath'])
                    self.assertEqual(asset['role'], 'asr_asset')
                    self.assertEqual(source['url'], old_source['url'])
                    self.assertEqual(source['archive']['memberPath'], 'asr/assets/haas-ja-v1.0-onnx-quantized/models/bpe.vocab')
                    self.assertEqual(source['archive']['expectedHash'], expected)
                else:
                    self.assertEqual(manifest['modelVersion'],'291488c8151be24d7da4bf7af26e533fad96e407')
                    self.assertFalse(any(f['relativePath'] == 'models/bpe.vocab' for f in manifest['files']))
            first = (root/'catalog.json').read_bytes()
            bundle.append_windows_choices(root)
            self.assertEqual((root/'catalog.json').read_bytes(), first)
            self.assertEqual(bundle.digest(root/'runtime.tar'), expected)

    def test_model_specific_asset_rejects_catalog_hash_mismatch(self):
        import copy
        definitions = copy.deepcopy(bundle.mon_model_catalog.definitions())
        haas = next(d for d in definitions if d['id'] == 'haas-ja-v1.0-onnx-quantized')
        next(f for f in haas['files'] if f['relativePath'] == 'models/bpe.vocab')['expectedHash'] = '0'*64
        with patch.object(bundle.mon_model_catalog, 'definitions', return_value=definitions):
            with self.assertRaisesRegex(ValueError, 'catalog SHA-256'):
                bundle.asset_members()
