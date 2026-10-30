import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import mon_windows_workers as workers


class WorkerExtractionTests(unittest.TestCase):
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
