import hashlib
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
                self.assertEqual(archive.getnames(), ['bin/mon-transducer-worker', 'notices/nlohmann-MIT.txt'])
            self.assertEqual(len(first['files']), 2)
            (root/'mon-transducer-worker').write_bytes(b'resigned native worker')
            second=bundle.assemble(root)
            self.assertNotEqual(first['archiveSha256'],second['archiveSha256'])
            self.assertNotEqual(first['files'][0]['expectedHash'],second['files'][0]['expectedHash'])

    def test_windows_catalog_uses_packaged_dlls_and_preserves_fixed_model_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            for path,data in [('workers/mon-transducer-worker.exe',b'Windows worker'),('workers/nlohmann-MIT.txt',b'MIT')]:
                file=root/path;file.parent.mkdir(parents=True,exist_ok=True);file.write_bytes(data)
            sherpa_members={'sherpa/sherpa-onnx-c-api.dll':b'signed C API','sherpa/onnxruntime.dll':b'signed ORT'}
            with tarfile.open(root/'runtime.tar','w') as archive:
                for member,data in {**sherpa_members,'asr/lib/sherpa-onnx-c-api.dll':b'old duplicate'}.items():
                    entry=tarfile.TarInfo(member);entry.size=len(data);archive.addfile(entry,io.BytesIO(data))
            windows_uri='mon-bundled://windows-runtime-v1/runtime.tar'
            sherpa_sources=[dict(relativePath=Path(member).name,url=windows_uri,
                                 archive=dict(format='tar',expectedSize=1,expectedHash='0'*64,memberPath=member))
                            for member in sherpa_members]
            (root/'catalog.json').write_text(json.dumps([dict(recipe=dict(manifest=dict(engine='sherpa-onnx'),sources=sherpa_sources))]))
            bundle.append_windows_choices(root)
            choices=json.loads((root/'catalog.json').read_text(encoding='utf8'))
            self.assertEqual([choice['id'] for choice in choices[1:]], [
                'windows-reazonspeech-k2-v2-int8-fp32', 'windows-reazonspeech-k2-v2-fp32',
            ])
            expected=bundle.digest(root/'runtime.tar')
            with tarfile.open(root/'runtime.tar') as archive:
                self.assertCountEqual(archive.getnames(), [*sherpa_members,'asr/bin/mon-transducer-worker.exe','asr/notices/nlohmann-MIT.txt'])
                for member,data in sherpa_members.items():
                    with archive.extractfile(member) as stream:
                        self.assertEqual(stream.read(),data)
            for choice in choices:
                for source in choice['recipe']['sources']:
                    if source['url']==windows_uri:
                        self.assertEqual(source['archive']['expectedHash'],expected)
                        self.assertEqual(source['archive']['expectedSize'],(root/'runtime.tar').stat().st_size)
            for choice in choices[1:]:
                manifest=choice['recipe']['manifest']
                self.assertEqual(manifest['target'],'windows_x86_64')
                self.assertEqual(manifest['backend']['files']['library'],'lib/sherpa-onnx-c-api.dll')
                self.assertEqual(len([f for f in manifest['files'] if f['role']=='executable']),1)
                files=[(file,source) for file,source in zip(manifest['files'],choice['recipe']['sources'],strict=True)
                       if file['role']=='runtime_library']
                self.assertCountEqual([source['archive']['memberPath'] for _,source in files],sherpa_members)
                for file,source in files:
                    member=source['archive']['memberPath']
                    self.assertEqual(file['relativePath'], f'lib/{Path(member).name}')
                    self.assertEqual(file['relativePath'],source['relativePath'])
                    self.assertEqual(file['expectedSize'],len(sherpa_members[member]))
                    self.assertEqual(file['expectedHash'],hashlib.sha256(sherpa_members[member]).hexdigest())
                self.assertEqual(manifest['backend']['id'], 'sherpa-onnx-transducer')
                self.assertEqual(manifest['modelVersion'], '291488c8151be24d7da4bf7af26e533fad96e407')
            first = (root/'catalog.json').read_bytes()
            bundle.append_windows_choices(root)
            self.assertEqual((root/'catalog.json').read_bytes(), first)
            self.assertEqual(bundle.digest(root/'runtime.tar'), expected)
