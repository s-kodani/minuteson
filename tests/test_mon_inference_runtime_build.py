"""Exercise cache reuse/publication without downloading or compiling engines."""
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
if sys.platform == 'win32':
    raise unittest.SkipTest('The macOS worker builder requires POSIX file locks')
import mon_asr_bundle as bundle
import mon_inference_runtime as runtime


class PersistentWorkerBuildTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        workers = self.root / 'probes/inference-worker'
        workers.mkdir(parents=True)
        (workers / 'worker.hpp').write_bytes(b'worker source')
        notices = self.root / 'third-party/notices'
        notices.mkdir(parents=True)
        (notices / 'spdx-MIT-template.txt').write_text('<year> <copyright holders> license')
        self.cache = self.root / 'cache'
        self.cmake = self.root / 'cmake'
        self.identity = {'arch': 'arm64', 'clang': 'fixed'}
        self.commands = []
        for mock in (patch.object(runtime, 'ROOT', self.root),
                     patch.object(runtime, 'build_identity', side_effect=lambda *_: dict(self.identity)),
                     patch.object(runtime, 'acquire', side_effect=self.acquire),
                     patch.object(runtime, 'extract', side_effect=self.extract),
                     patch.object(runtime, 'patch_whisper', side_effect=self.patch_source),
                     patch.object(runtime, 'patch_llama', side_effect=self.patch_source),
                     patch.object(runtime.asr, 'command', side_effect=self.command),
                     patch.object(runtime.asr, 'verify_binary'),
                     patch.object(bundle, 'assemble', side_effect=self.assemble)):
            mock.start()
            self.addCleanup(mock.stop)

    @staticmethod
    def acquire(cache, name, *_args, **_kwargs):
        path = cache / name
        if not path.exists():
            path.write_bytes(b'fixed upstream bytes')
        return path

    @staticmethod
    def extract(_archive, destination):
        root = destination / 'upstream'
        root.mkdir(parents=True)
        (root / 'CMakeLists.txt').write_text('upstream\n')
        (root / 'LICENSE').write_text('upstream license')
        (root / 'vendor/nlohmann').mkdir(parents=True)
        (root / 'vendor/nlohmann/json.hpp').write_text('JSON notice\n#ifndef HEADER\n')
        return root

    @staticmethod
    def patch_source(root):
        path = root / 'CMakeLists.txt'
        text = path.read_text()
        if 'patched' in text:
            raise AssertionError('source was patched twice')
        path.write_text(text + 'patched\n')

    @staticmethod
    def assemble(package):
        (package / 'asr-runtime.tar').write_bytes(b'assembled package')

    def command(self, arguments, **_kwargs):
        self.commands.append(arguments)
        if arguments[0] == str(self.cmake):
            if '--build' in arguments:
                build = Path(arguments[2])
                targets = arguments[arguments.index('--target') + 1:arguments.index('--parallel')]
                for target in targets:
                    path = build / 'bin' / target
                    path.parent.mkdir(parents=True, exist_ok=True)
                    if not path.exists():
                        path.write_bytes(b'compiler output: ' + target.encode())
            else:
                Path(arguments[arguments.index('-B') + 1]).mkdir(parents=True, exist_ok=True)
        elif arguments[0] == '/usr/bin/codesign':
            path = Path(arguments[-1])
            path.write_bytes(path.read_bytes() + b' signed')
        return b'', b''

    def prepare(self):
        return runtime.prepare(self.cache, self.cmake)

    def test_restored_intermediates_keep_source_mtime_and_regenerate_checked_signed_package(self):
        package = self.prepare()
        source = next((self.cache / 'intermediates').glob('*/sources/whisper/upstream/CMakeLists.txt'))
        os.utime(source, ns=(1_000_000_000, 1_000_000_000))
        compiled = next((self.cache / 'intermediates').glob('*/build/llama/bin/mon-llama-worker'))
        unsigned = compiled.read_bytes()
        fingerprint = json.loads((package / 'build-info.json').read_text())['buildFingerprint']
        shutil.rmtree(package)  # Actions restores intermediates, never package/.
        self.prepare()
        self.assertEqual(source.stat().st_mtime_ns, 1_000_000_000)
        self.assertEqual(compiled.read_bytes(), unsigned)
        self.assertFalse(unsigned.endswith(b' signed'))
        self.assertEqual((package / 'mon-llama-worker').read_bytes(), unsigned + b' signed')
        self.assertEqual(json.loads((package / 'build-info.json').read_text())['buildFingerprint'], fingerprint)
        self.assertEqual(sum('--build' in command for command in self.commands), 4)
        self.assertEqual(sum('-S' in command for command in self.commands), 4)
        self.assertEqual(sum(command[0] == '/usr/bin/codesign' for command in self.commands), 8)
        self.assertEqual(len(list((self.cache / 'intermediates').iterdir())), 1)
        self.assertTrue(all(path.is_file() for path in (self.cache / 'downloads').iterdir()))

    def test_changed_toolchain_uses_a_different_source_and_build_tree(self):
        self.prepare()
        self.identity['clang'] = 'upgraded'
        self.prepare()
        self.assertEqual(len(list((self.cache / 'intermediates').iterdir())), 2)

    def test_failed_assembly_keeps_previous_package_bytes(self):
        package = self.prepare()
        before = {path.name: path.read_bytes() for path in package.iterdir()}
        def fail_assembly(pending):
            (pending / 'asr-runtime.tar').write_bytes(b'partial archive')
            raise ValueError('synthetic assembly failure')
        with patch.object(bundle, 'assemble', side_effect=fail_assembly):
            with self.assertRaisesRegex(ValueError, 'assembly failure'):
                self.prepare()
        self.assertEqual({path.name: path.read_bytes() for path in package.iterdir()}, before)
        self.assertFalse(list(self.cache.glob('package-prepare-*')))

    def test_failed_patch_is_not_published_as_reusable_source(self):
        with patch.object(runtime, 'patch_llama', side_effect=ValueError('patch failed')):
            with self.assertRaisesRegex(ValueError, 'patch failed'):
                self.prepare()
        self.assertFalse(list((self.cache / 'intermediates').glob('*/sources')))
        self.assertFalse(list(self.cache.glob('source-prepare-*')))
        self.prepare()

    def test_interrupted_publication_restores_previous_package(self):
        package = self.root / 'published'; package.mkdir()
        (package / 'worker').write_bytes(b'previous')
        pending = self.root / 'pending'; pending.mkdir()
        (pending / 'worker').write_bytes(b'new')
        rename = Path.rename
        def interrupt(path, destination):
            if path == pending:
                raise KeyboardInterrupt()
            return rename(path, destination)
        with patch.object(Path, 'rename', interrupt):
            with self.assertRaises(KeyboardInterrupt):
                runtime.publish_package(pending, package)
        self.assertEqual((package / 'worker').read_bytes(), b'previous')


class WorkerToolchainTests(unittest.TestCase):
    def test_identity_queries_the_actual_clean_build_environment_and_flags(self):
        calls = []
        def command(arguments, **kwargs):
            calls.append((arguments, kwargs['env']))
            return (' '.join(arguments).encode(), b'')
        with patch.object(runtime.asr, 'command', side_effect=command), \
                patch.dict(os.environ, {'DEVELOPER_DIR': '/untrusted/compiler', 'CXXFLAGS': '-unexpected'}):
            identity = runtime.build_identity(Path('/usr/bin/cmake'), Path('/private/tmp/cache'))
        self.assertTrue(any(arguments[0] == '/usr/bin/clang' for arguments, _ in calls))
        self.assertTrue(any('--show-sdk-version' in arguments for arguments, _ in calls))
        self.assertTrue(all('DEVELOPER_DIR' not in env and 'CXXFLAGS' not in env for _, env in calls))
        original = runtime.build_fingerprint('recipe', identity)
        for field in ('clang', 'sdk', 'cmake', 'cache', 'options'):
            self.assertNotEqual(runtime.build_fingerprint('recipe', {**identity, field: 'changed'}), original)
        self.assertNotEqual(runtime.build_fingerprint('changed recipe', identity), original)


if __name__ == '__main__':
    unittest.main()
