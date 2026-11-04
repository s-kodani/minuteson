import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
import zipfile
from types import SimpleNamespace
from contextlib import redirect_stderr
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('mon_windows_runtime_test', ROOT / 'tools/mon_windows_runtime.py')
runtime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


class WindowsRuntimeRedistributableTests(unittest.TestCase):
    def test_worker_destination_is_explicit_even_for_a_separately_imported_builder(self):
        import sys
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);package=root/'package';package.mkdir();(package/'old.txt').write_bytes(b'old')
            destinations=[]
            def workers(*,package_root,vulkan=False):
                destinations.append(package_root)
                self.assertNotEqual(package_root,package)
                (package_root/'workers').mkdir(exist_ok=True)
                (package_root/'workers/mon-transducer-worker.exe').write_bytes(b'fixed worker')
            def append(package_root):
                self.assertEqual((package_root/'workers/mon-transducer-worker.exe').read_bytes(),b'fixed worker')
            modules={'mon_windows_workers':SimpleNamespace(prepare=workers),'mon_asr_bundle':SimpleNamespace(append_windows_choices=append)}
            with patch.dict(sys.modules,modules),patch.object(runtime,'os',SimpleNamespace(name='nt')),patch.object(runtime,'CACHE',root),patch.object(runtime,'PACKAGE',package),patch.object(runtime,'visual_cpp_runtime_files',return_value={}),patch.object(runtime,'build_whisper'),patch.object(runtime,'upstream_engines'),patch.object(runtime,'catalog'),patch.object(runtime,'build_embedding'):
                runtime.main()
            self.assertEqual(len(destinations),2)
            self.assertEqual((package/'workers/mon-transducer-worker.exe').read_bytes(),b'fixed worker')
            self.assertFalse((package/'old.txt').exists())

    def test_fixed_archives_replace_entire_engine_subtrees_and_filter_installer_dlls(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);package=root/'package';archive=root/'llama.zip';sherpa=root/'sherpa.tar'
            for engine in ('llama','llama11960'):
                (package/engine).mkdir(parents=True);(package/engine/'obsolete.dll').write_bytes(b'old vulnerable runtime')
            with zipfile.ZipFile(archive,'w') as z:
                for name in ('llama-completion.exe','ggml-base.dll','ggml-rpc.dll','rpc-server.exe','LICENSE'):
                    z.writestr(name,b'fixed '+name.encode())
            with tarfile.open(sherpa,'w') as tar:
                data=b'sherpa';entry=tarfile.TarInfo('sherpa/sherpa-onnx-offline-speaker-diarization.exe');entry.size=len(data);tar.addfile(entry,io.BytesIO(data))
            def acquire(url,*_):
                self.assertNotIn('/b11533/',url)
                return archive if '/llama.cpp/' in url else sherpa
            with patch.object(runtime,'PACKAGE',package),patch.object(runtime,'acquire',side_effect=acquire),patch.object(runtime.mon_model_catalog,'compile_target',return_value=[]):
                runtime.upstream_engines()
            for engine in ('llama','llama11960'):
                self.assertEqual({p.relative_to(package/engine).as_posix() for p in (package/engine).rglob('*') if p.is_file()},{'llama-completion.exe','ggml-base.dll','notices/llama-MIT.txt'})
                self.assertEqual((package/engine/'llama-completion.exe').read_bytes(),b'fixed llama-completion.exe')

    def test_failure_in_new_staging_preserves_published_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);package=root/'package';package.mkdir();(package/'runtime.tar').write_bytes(b'previous package')
            with patch.object(runtime,'os',SimpleNamespace(name='nt')),patch.object(runtime,'CACHE',root),patch.object(runtime,'PACKAGE',package),patch.object(runtime,'visual_cpp_runtime_files',return_value={}),patch.object(runtime,'build_whisper',side_effect=ValueError('synthetic staging failure')):
                with self.assertRaisesRegex(ValueError,'staging failure'):runtime.main()
                self.assertEqual(runtime.PACKAGE,package)
            self.assertEqual((package/'runtime.tar').read_bytes(),b'previous package')
            self.assertEqual(list(root.iterdir()),[package])

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.program64 = self.root / 'Program Files'
        self.program32 = self.root / 'Program Files (x86)'
        environment = {'ProgramFiles': str(self.program64), 'ProgramW6432': str(self.program64),
                       'ProgramFiles(x86)': str(self.program32)}
        environment_patch = patch.dict(runtime.os.environ, environment)
        environment_patch.start()
        self.addCleanup(environment_patch.stop)
        path_patch = patch.object(runtime.shutil, 'which', return_value=None)
        path_patch.start()
        self.addCleanup(path_patch.stop)

    def installation(self, root=None, edition='Enterprise'):
        return (root or self.program64) / 'Microsoft Visual Studio/2022' / edition

    def redist(self, installation=None, version='14.44.35112', missing=(), architecture='x64'):
        directory = (installation or self.installation()) / 'VC/Redist/MSVC' / version / architecture / 'Microsoft.VC143.CRT'
        directory.mkdir(parents=True)
        for name in runtime.VC_RUNTIME_FILES:
            if name not in missing:
                (directory / name).write_bytes((version + ':' + name).encode())
        return directory

    def vswhere(self):
        executable = self.program32 / 'Microsoft Visual Studio/Installer/vswhere.exe'
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b'synthetic locator; never executed')
        return executable

    def test_standard_64_bit_enterprise_installation_is_found(self):
        expected = self.redist()
        self.assertEqual(runtime.visual_cpp_redistributable(), expected)

    def test_x86_program_files_build_tools_installation_is_found(self):
        expected = self.redist(self.installation(self.program32, 'BuildTools'))
        self.assertEqual(runtime.visual_cpp_redistributable(), expected)

    def test_program_w6432_is_searched_when_program_files_points_to_x86(self):
        expected = self.redist()
        with patch.dict(runtime.os.environ, {'ProgramFiles': str(self.program32)}):
            self.assertEqual(runtime.visual_cpp_redistributable(), expected)

    def test_vswhere_finds_custom_installation_and_includes_build_tools(self):
        locator = self.vswhere()
        installation = self.root / 'custom VS install'
        expected = self.redist(installation)
        with patch.object(runtime.subprocess, 'run', return_value=subprocess.CompletedProcess(
                [], 0, json.dumps([{'installationPath': str(installation)}]))) as run:
            self.assertEqual(runtime.visual_cpp_redistributable(), expected)
        run.assert_called_once_with([str(locator), '-products', '*', '-version', '[17.0,18.0)',
            '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-format', 'json', '-utf8'],
            capture_output=True, text=True, encoding='utf-8-sig', check=True, timeout=30)

    def test_complete_x64_crt_with_newest_numeric_version_is_selected(self):
        self.redist(version='14.9.99999')
        expected = self.redist(version='14.10.1')
        self.redist(version='14.99.1', missing=('msvcp140.dll',))
        self.redist(version='99.0.1', architecture='x86')
        self.assertEqual(runtime.visual_cpp_redistributable(), expected)

    def test_v143_alias_can_supply_a_complete_crt(self):
        expected = self.redist(version='v143')
        self.assertEqual(runtime.visual_cpp_redistributable(), expected)

    def test_failed_or_malformed_vswhere_can_fall_back_to_standard_installation(self):
        self.vswhere()
        expected = self.redist()
        for result in (subprocess.CalledProcessError(1, ['vswhere']),
                       subprocess.CompletedProcess([], 0, '{malformed'),
                       subprocess.CompletedProcess([], 0, '{}')):
            errors = io.StringIO()
            keyword = {'side_effect': result} if isinstance(result, Exception) else {'return_value': result}
            with self.subTest(result=result), patch.object(runtime.subprocess, 'run', **keyword), redirect_stderr(errors):
                self.assertEqual(runtime.visual_cpp_redistributable(), expected)
            self.assertIn('Visual Studio discovery warning', errors.getvalue())

    def test_incomplete_or_empty_crt_is_rejected_without_omitting_required_files(self):
        crt = self.redist(missing=('vcruntime140_1.dll',))
        with self.assertRaisesRegex(ValueError, 'vcruntime140_1.dll'):
            runtime.visual_cpp_redistributable()
        (crt / 'vcruntime140_1.dll').write_bytes(b'')
        with self.assertRaisesRegex(ValueError, 'redistributable directory missing'):
            runtime.visual_cpp_redistributable()

    def test_redist_preflight_failure_preserves_previous_package_before_builds(self):
        package = self.root / 'package'
        package.mkdir()
        previous = package / 'runtime.tar'
        previous.write_bytes(b'previous verified runtime')
        with patch.object(runtime.os, 'name', 'nt'), patch.object(runtime, 'PACKAGE', package), \
                patch.object(runtime, 'visual_cpp_runtime_files', side_effect=ValueError('missing VC runtime')), \
                patch.object(runtime, 'build_whisper') as whisper, patch.object(runtime, 'upstream_engines') as upstream, \
                patch.object(runtime, 'catalog') as catalog, patch.object(runtime, 'build_embedding') as embedding:
            with self.assertRaisesRegex(ValueError, 'missing VC runtime'):
                runtime.main()
        for build in (whisper, upstream, catalog, embedding):
            build.assert_not_called()
        self.assertEqual(list(package.iterdir()), [previous])
        self.assertEqual(previous.read_bytes(), b'previous verified runtime')

    def test_read_failure_does_not_partially_replace_existing_package(self):
        crt = self.redist()
        package = self.root / 'package'
        for engine in ('llama', 'llama11960'):
            (package / engine).mkdir(parents=True)
            for name in runtime.VC_RUNTIME_FILES:
                (package / engine / name).write_bytes(b'previous DLL')
        (package / 'runtime.tar').write_bytes(b'previous archive')
        before = {p.relative_to(package): p.read_bytes() for p in package.rglob('*') if p.is_file()}
        read = Path.read_bytes
        def fail_last_dll(path):
            if path == crt / runtime.VC_RUNTIME_FILES[-1]:
                raise PermissionError('synthetic unreadable CRT')
            return read(path)
        with patch.object(runtime, 'PACKAGE', package), \
                patch.object(runtime, 'visual_cpp_redistributable', return_value=crt), \
                patch.object(Path, 'read_bytes', fail_last_dll):
            with self.assertRaisesRegex(PermissionError, 'unreadable CRT'):
                runtime.catalog()
        after = {p.relative_to(package): p.read_bytes() for p in package.rglob('*') if p.is_file()}
        self.assertEqual(after, before)

    def test_catalog_keeps_every_required_dll_in_both_llama_packages_and_hashes(self):
        crt = self.redist()
        payload = {name: (crt / name).read_bytes() for name in runtime.VC_RUNTIME_FILES}
        package = self.root / 'package'
        executables = {'whisper': 'whisper-cli.exe', 'llama': 'llama-completion.exe',
                       'llama11960': 'llama-completion.exe', 'sherpa': 'sherpa-onnx-offline-speaker-diarization.exe'}
        for engine, name in executables.items():
            (package / engine).mkdir(parents=True)
            (package / engine / name).write_bytes(engine.encode())
        with patch.object(runtime, 'PACKAGE', package), patch.object(runtime, 'sign'):
            runtime.catalog(payload)
        with tarfile.open(package / 'runtime.tar') as archive:
            for engine in ('llama', 'llama11960'):
                for name, data in payload.items():
                    self.assertEqual((package / engine / name).read_bytes(), data)
                    self.assertEqual(archive.extractfile(engine + '/' + name).read(), data)
        choices = json.loads((package / 'catalog.json').read_text(encoding='utf8'))
        for choice in choices:
            manifest = choice['recipe']['manifest']
            if manifest['engine'] != 'llama.cpp':
                continue
            files = {entry['relativePath']: entry for entry in manifest['files']}
            for name in runtime.VC_RUNTIME_FILES:
                self.assertEqual(files[name]['role'], 'runtime_library')
                self.assertEqual(files[name]['expectedHash'], runtime.digest(crt / name))

    def test_supplied_runtime_payload_cannot_omit_a_required_dll(self):
        with patch.object(runtime, 'dump') as dump:
            with self.assertRaisesRegex(ValueError, 'All required'):
                runtime.catalog({'vcruntime140.dll': b'incomplete'})
        dump.assert_not_called()


if __name__ == '__main__':
    unittest.main()
