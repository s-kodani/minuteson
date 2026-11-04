import io
import hashlib
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import mon_windows_workers as workers


class WorkerExtractionTests(unittest.TestCase):
    @unittest.skipIf(os.name == 'nt', 'directory links require Windows privileges')
    def test_sync_rejects_linked_destination_parent_before_copying(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            source = root / 'source'; source.mkdir()
            (source / 'worker.hpp').write_bytes(b'input')
            external = root / 'other'; external.mkdir()
            linked = root / 'linked'; linked.symlink_to(external, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, 'links'):
                workers.sync_worker_sources(source, linked / 'mon')
            self.assertEqual(list(external.iterdir()), [])

    def test_identical_checkout_preserves_compiler_input_mtime_and_syncs_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            source = root / 'checkout'; source.mkdir()
            destination = root / 'cached/mon'; destination.mkdir(parents=True)
            (source / 'worker.hpp').write_bytes(b'unchanged header')
            (source / 'new.cpp').write_bytes(b'added source')
            cached = destination / 'worker.hpp'; cached.write_bytes(b'unchanged header')
            os.utime(cached, ns=(1_000_000_000, 1_000_000_000))
            (destination / 'obsolete.cpp').write_bytes(b'removed source')
            (destination / 'new.cpp').write_bytes(b'old source')
            (destination / 'json.hpp').write_bytes(b'upstream JSON header')
            (destination / 'c-api.h').write_bytes(b'upstream C API')
            (destination / '.patched').write_bytes(b'patch marker')
            workers.sync_worker_sources(source, destination)
            self.assertEqual(cached.stat().st_mtime_ns, 1_000_000_000)
            self.assertEqual((destination / 'new.cpp').read_bytes(), b'added source')
            self.assertFalse((destination / 'obsolete.cpp').exists())
            self.assertEqual((destination / 'json.hpp').read_bytes(), b'upstream JSON header')
            self.assertEqual((destination / 'c-api.h').read_bytes(), b'upstream C API')
            self.assertEqual((destination / '.patched').read_bytes(), b'patch marker')
            (source / 'worker.hpp').write_bytes(b'changed header')
            workers.sync_worker_sources(source, destination)
            self.assertEqual(cached.read_bytes(), b'changed header')

    def test_generated_nlohmann_notice_matches_the_offline_license_policy_bytes(self):
        policy = json.loads((ROOT / 'app/model-catalog/model-license-policy.json').read_text(encoding='utf8'))
        record = next(n for n in policy['notices'] if n['name'] == 'nlohmann-MIT.txt')
        expected = (ROOT / 'third-party/model-licenses' / record['file']).read_bytes()
        license_text = (ROOT / 'third-party/notices/spdx-MIT-template.txt').read_text(encoding='utf8').replace('<year> <copyright holders>', '2013 - 2025 Niels Lohmann')
        self.assertTrue(expected.endswith(('\n' + license_text).encode('utf8')))
        header = expected.decode('utf8').removesuffix('\n' + license_text)
        with tempfile.TemporaryDirectory() as temporary:
            package = Path(temporary)
            # Replace an old Windows-generated copy as well as creating a new one.
            (package / 'nlohmann-MIT.txt').write_bytes(expected.replace(b'\n', b'\r\n'))
            workers.write_nlohmann_notice(package, header)
            actual = (package / 'nlohmann-MIT.txt').read_bytes()
        self.assertEqual(actual, expected)
        self.assertEqual(len(actual), record['bytes'])
        self.assertEqual(hashlib.sha256(actual).hexdigest(), record['sha256'])

    def test_native_sources_extract_without_unused_long_web_ui_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / 'llama.tar'
            ui_path = ('tools/ui/src/lib/components/app/models/discover/'
                       'ModelsDiscoverDetails/ModelsDiscoverDetailsDownloadOptions/'
                       'ModelsDiscoverDetailsDownloadOptionsDraftDownloadButton.svelte')
            files = {'LICENSE': b'license', 'src/llama.cpp': b'native source',
                     'tools/cli/cli.cpp': b'cli source', ui_path: b'unused web UI'}
            with tarfile.open(archive, 'w') as output:
                for name, data in files.items():
                    member = tarfile.TarInfo('llama-fixed-commit/' + name)
                    member.size = len(data)
                    output.addfile(member, io.BytesIO(data))
            target = root / 'workers-source' / 'llama'
            tree = workers.extract(archive, target)
            self.assertEqual(tree, target / 'tree')
            for name in ('LICENSE', 'src/llama.cpp', 'tools/cli/cli.cpp'):
                self.assertEqual((tree / name).read_bytes(), files[name])
            self.assertFalse((tree / 'tools/ui').exists())
            self.assertTrue((target / '.complete').is_file())


if __name__ == '__main__':
    unittest.main()
