import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('mon_ci_cache', ROOT / 'tools/mon_ci_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class NativeCacheTests(unittest.TestCase):
    def test_sources_build_options_and_exact_toolchain_invalidate_the_key(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            files = ['Cargo.lock', 'tools/mon_windows_workers.py',
                     'app/model-catalog/whisper-runtime-build.json',
                     'probes/inference-worker/llama-main.cpp',
                     'probes/windows-capture/CMakeLists.txt',
                     'crates/mon-core/src/lib.rs', 'probes/macos-capture/Sources/Core.swift']
            for name in files:
                path = root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'original')
            identity = {'platform': 'win32', 'arch': 'AMD64', 'rust': 'fixed', 'image': 'fixed'}
            original = cache.cache_key(root, identity)
            self.assertEqual(cache.cache_key(root, identity), original)
            for name in files:
                path = root / name; path.write_bytes(b'changed')
                self.assertNotEqual(cache.cache_key(root, identity), original, name)
                path.write_bytes(b'original')
            for field in identity:
                self.assertNotEqual(cache.cache_key(root, {**identity, field: 'different'}), original, field)

    def test_generated_packages_and_distribution_outputs_are_not_native_cache_inputs_or_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            identity = {'platform': 'darwin', 'arch': 'arm64'}
            before = cache.cache_key(root, identity)
            for name in ['.cache/windows-runtime/package/runtime.tar', 'output/setup.exe',
                         'probes/windows-capture/build/Release/probe.exe']:
                path = root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'generated')
            self.assertEqual(cache.cache_key(root, identity), before)
        for host in ('win32', 'darwin'):
            paths = cache.build_paths(host)
            self.assertTrue(paths)
            self.assertFalse(any('package' in p or 'output/' in p or 'sign' in p for p in paths))


if __name__ == '__main__':
    unittest.main()
