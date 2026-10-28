import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

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
                self.assertEqual(archive.getnames(),['bin/mon-transducer-worker','notices/nlohmann-MIT.txt'])
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
            choices=json.loads((root/'catalog.json').read_text())
            self.assertEqual(len(choices),3)
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
                self.assertEqual(manifest['modelVersion'],'291488c8151be24d7da4bf7af26e533fad96e407')
