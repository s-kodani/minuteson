import io
import hashlib
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import mon_windows_workers as workers


class WorkerExtractionTests(unittest.TestCase):
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
