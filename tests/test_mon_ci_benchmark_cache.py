"""Native cache trials must restore compiler products without touching sources."""
import copy
import hashlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import mon_ci_benchmark as benchmark


class NativeCacheBenchmarkTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name).resolve()
        self.root = self.directory / 'workspace'; self.root.mkdir()
        self.runner = self.directory / 'runner'; self.runner.mkdir()
        self.definition = SimpleNamespace(build_paths=lambda component: list(benchmark.cache_scopes(component)))

    def put(self, name, data=b'compiler output'):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def test_swift_relative_alias_is_archived_literally_and_restored_with_mtimes(self):
        binary = self.put('probes/macos-capture/.build/arm64/release/probe')
        os.utime(binary, ns=(1_000_000_000, 1_000_000_000))
        alias = binary.parents[2] / 'release'
        alias.symlink_to('arm64/release', target_is_directory=True)
        archive = self.runner / 'capture.tar'
        benchmark.cache_snapshot(self.root, 'macos-capture', self.definition, archive)
        with tarfile.open(archive) as source:
            member = source.getmember('probes/macos-capture/.build/release')
            self.assertTrue(member.issym())
            self.assertEqual(member.linkname, 'arm64/release')
        benchmark.cache_remove_components(self.root, ['macos-capture'], self.definition)
        benchmark.cache_restore(self.root, 'macos-capture', self.definition, archive)
        self.assertTrue(alias.is_symlink())
        self.assertEqual((alias / 'probe').read_bytes(), b'compiler output')
        self.assertEqual(binary.stat().st_mtime_ns, 1_000_000_000)

    def test_traversal_members_and_links_outside_component_are_rejected_before_extract(self):
        protected = self.put('app/source.rs', b'frozen source')
        for name, target in [('probes/macos-capture/.build/../../../../app/source.rs', None),
                             ('probes/macos-capture/.build/release', '../../../app')]:
            with self.subTest(name=name, target=target):
                archive = self.runner / 'unsafe.tar'
                with tarfile.open(archive, 'w') as output:
                    member = tarfile.TarInfo(name)
                    if target:
                        member.type = tarfile.SYMTYPE; member.linkname = target
                        output.addfile(member)
                    else:
                        member.size = 3; output.addfile(member, io.BytesIO(b'bad'))
                with self.assertRaises(ValueError):
                    benchmark.cache_restore(self.root, 'macos-capture', self.definition, archive)
                self.assertEqual(protected.read_bytes(), b'frozen source')

    def test_changed_allowlist_and_linked_ancestor_cannot_trigger_cleanup(self):
        protected = self.put('app/source.rs', b'frozen source')
        changed = SimpleNamespace(build_paths=lambda _: ['app'])
        with self.assertRaisesRegex(ValueError, 'allowlist'):
            benchmark.cache_remove_components(self.root, ['macos-workers'], changed)
        self.assertEqual(protected.read_bytes(), b'frozen source')
        other = self.directory / 'other'; other.mkdir()
        (other / 'sentinel').write_bytes(b'keep')
        (self.root / '.cache').symlink_to(other, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'linked parent'):
            benchmark.cache_remove_components(self.root, ['macos-workers'], self.definition)
        self.assertEqual((other / 'sentinel').read_bytes(), b'keep')

    def windows_modules(self):
        return {'mon_windows_preparation': SimpleNamespace(
                    OUTPUT_TREES=('app/dist', '.cache/windows-runtime/package', '.cache/windows-third-party-notices'),
                    OUTPUT_FILES=('app/src-tauri/binaries/mon-windows-capture-x86_64-pc-windows-msvc.exe',)),
                'mon_windows_vulkan_sdk': SimpleNamespace(VERSION='1.4.328.1')}

    def test_generated_cleanup_keeps_restored_capture_helper_and_other_sdk_versions(self):
        helper = self.put('probes/windows-capture/build/Release/mon-windows-capture.exe', b'restored helper')
        protected = self.put('app/src/source.rs', b'frozen source')
        other = self.put('.cache/windows-tools/VulkanSDK-other/keep', b'other tools')
        pinned = self.put('.cache/windows-tools/VulkanSDK-1.4.328.1/Bin/glslc.exe')
        staged = self.put('app/src-tauri/binaries/mon-windows-capture-x86_64-pc-windows-msvc.exe')
        package = self.put('.cache/windows-runtime/package/runtime.tar')
        with patch.dict(sys.modules, self.windows_modules()):
            benchmark.cache_clean_generated(self.root, 'win32')
        self.assertEqual(helper.read_bytes(), b'restored helper')
        self.assertEqual(protected.read_bytes(), b'frozen source')
        self.assertEqual(other.read_bytes(), b'other tools')
        self.assertFalse(pinned.exists()); self.assertFalse(staged.exists()); self.assertFalse(package.exists())

    def test_cold_then_three_restores_use_exact_preparation_and_clear_receipt_environment(self):
        source = self.put('tools/frozen.py', b'frozen source')
        original_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        self.definition.input_files = lambda root, component: [source]
        self.definition.toolchain_identity = lambda component: {'compiler': 'fixed'}
        self.definition.cache_key = lambda root, identity, component: component + hashlib.sha256(source.read_bytes()).hexdigest()
        calls, checkpoints, report = [], [], {}
        def prepare(arguments, *, cwd, env, timeout):
            self.assertEqual(arguments, ['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(self.root / 'app/prepare-windows.ps1')])
            self.assertEqual(cwd, self.root)
            self.assertNotIn('MON_WINDOWS_PREPARATION_RECEIPT', env)
            self.assertNotIn('TAURI_CONFIG', env)
            self.assertEqual(env['MON_WINDOWS_RELEASE_SIGNING'], '0')
            intermediate = self.root / '.cache/windows-runtime/whisper-build/object.obj'
            helper = self.root / 'probes/windows-capture/build/Release/mon-windows-capture.exe'
            if calls:
                self.assertEqual(intermediate.read_bytes(), b'cold intermediate')
                self.assertEqual(helper.read_bytes(), b'cold capture helper')
            self.put('.cache/windows-runtime/whisper-build/object.obj', b'cold intermediate')
            self.put('probes/windows-capture/build/Release/mon-windows-capture.exe', b'cold capture helper')
            self.put('.cache/windows-runtime/package/runtime.tar')
            self.put('app/dist/index.html')
            calls.append(arguments)
            return 0.01
        modules = {**self.windows_modules(), 'mon_ci_cache': self.definition}
        with patch.dict(sys.modules, modules), patch.object(sys, 'platform', 'win32'), \
                patch.dict(os.environ, {'RUNNER_TEMP': str(self.runner), 'MON_WINDOWS_PREPARATION_RECEIPT': 'stale receipt', 'TAURI_CONFIG': 'stale config', 'MON_WINDOWS_RELEASE_SIGNING': '1'}), \
                patch.object(subprocess, 'check_output', return_value=b'tools/frozen.py\0'):
            benchmark.cache_benchmark(self.root, report, lambda: checkpoints.append(copy.deepcopy(report)), prepare)
        self.assertEqual(len(calls), 4)
        value = report['cache']
        self.assertEqual(value['kind'], 'same-job-intermediate-round-trip')
        self.assertIsNone(value['actionsCacheHit'])
        self.assertTrue(value['simulatedCheckoutMtimes'])
        self.assertIn('combined preparation reuse', value['comparisonMeaning'])
        self.assertIn('isolated compilation-cache speedup', value['unsupportedInferences'])
        self.assertEqual([row['trial'] for row in value['trials']], ['cold', 'warm-1', 'warm-2', 'warm-3'])
        self.assertIn('as found', value['trials'][0]['dependencyState'])
        self.assertTrue(all('retained after initial' in row['dependencyState'] for row in value['trials'][1:]))
        self.assertTrue(all(row['status'] == 'success' for row in value['trials']))
        self.assertTrue(all(len(row['restoreComponents']) == 2 for row in value['trials'][1:]))
        self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), original_hash)
        self.assertEqual(list(self.runner.iterdir()), [])
        self.assertTrue(checkpoints)


if __name__ == '__main__':
    unittest.main()
