import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('mon_ci_cache', ROOT / 'tools/mon_ci_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class NativeCacheTests(unittest.TestCase):
    def test_only_components_that_compile_an_input_are_invalidated(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            captures = {'windows-capture', 'macos-capture'}
            workers = {'windows-runtime', 'macos-workers'}
            inputs = {'Cargo.lock': captures,
                      'tools/mon_windows_workers.py': {'windows-runtime'},
                      'tools/mon_whisper_runtime.py': workers,
                      'app/model-catalog/whisper-runtime-build.json': {'windows-runtime'},
                      'probes/inference-worker/llama-main.cpp': workers,
                      'probes/windows-capture/CMakeLists.txt': {'windows-capture'},
                      'crates/mon-core/src/lib.rs': captures,
                      'probes/macos-capture/Sources/Core.swift': {'macos-capture'},
                      'app/src-tauri/src/lib.rs': set(),
                      'crates/mon-native/src/lib.rs': set()}
            for name in inputs:
                path = root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'original')
            identity = {'platform': 'win32', 'arch': 'AMD64', 'rust': 'fixed', 'image': 'fixed'}
            original = {component: cache.cache_key(root, identity, component) for component in cache.COMPONENTS}
            self.assertEqual(len(set(original.values())), len(original))
            for name, affected in inputs.items():
                path = root / name; path.write_bytes(b'changed')
                for component in cache.COMPONENTS:
                    self.assertEqual(cache.cache_key(root, identity, component) != original[component],
                                     component in affected, (name, component))
                path.write_bytes(b'original')
            for field in identity:
                for component in cache.COMPONENTS:
                    self.assertNotEqual(cache.cache_key(root, {**identity, field: 'different'}, component),
                                        original[component], (field, component))

    def test_generated_packages_and_distribution_outputs_are_not_native_cache_inputs_or_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            identity = {'platform': 'darwin', 'arch': 'arm64'}
            before = {component: cache.cache_key(root, identity, component) for component in cache.COMPONENTS}
            for name in ['.cache/windows-runtime/package/runtime.tar', 'output/setup.exe',
                         '.cache/inference-runtime/package/mon-llama-worker',
                         '.cache/llama-runtime-check/assets/model',
                         'probes/windows-capture/build/Release/probe.exe']:
                path = root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'generated')
            for component in cache.COMPONENTS:
                self.assertEqual(cache.cache_key(root, identity, component), before[component])
        for component in cache.COMPONENTS:
            paths = cache.build_paths(component)
            self.assertTrue(paths)
            self.assertFalse(any('package' in p or 'output/' in p or 'sign' in p or 'llama-runtime-check' in p for p in paths))

    def test_removed_compiler_input_changes_the_exact_key(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / 'probes/inference-worker/removed.cpp'
            path.parent.mkdir(parents=True)
            path.write_bytes(b'source')
            identity = {'arch': 'arm64'}
            before = cache.cache_key(root, identity, 'macos-workers')
            path.unlink()
            self.assertNotEqual(cache.cache_key(root, identity, 'macos-workers'), before)


if __name__ == '__main__':
    unittest.main()
