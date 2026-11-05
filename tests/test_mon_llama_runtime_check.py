import hashlib
import io
import json
from pathlib import Path
import stat
import sys
import subprocess
import tarfile
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools'))
import mon_windows_parity_assets as assets
import mon_llama_runtime_check as native


class NativeOutputTests(unittest.TestCase):
    def test_only_exact_json_booleans_and_existing_end_marker_are_accepted(self):
        for expected in (True, False):
            for suffix in ('', '\n', '\n[end of text]\n'):
                path = Mock()
                path.read_text.return_value = '  ' + json.dumps({'ok': expected}) + suffix
                native.result(path, expected)
            for value in (1, 0, 1.0, None, 'true', [], {}, not expected):
                path = Mock()
                path.read_text.return_value = json.dumps({'ok': value})
                with self.subTest(expected=expected, value=value), self.assertRaises(ValueError):
                    native.result(path, expected)
        for raw in ('{}', '{"ok":true,"extra":0}', 'true', '{"ok":true} trailing'):
            path = Mock()
            path.read_text.return_value = raw
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                native.result(path, True)

    def test_direct_and_all_three_resident_results_are_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            model = root / 'model.gguf'; model.write_bytes(b'synthetic')
            choice = dict(id='llama-b11533-qwen3-4b-q4km', recipe=dict(manifest=dict(
                engineVersion='b11960', files=[dict(role='llm', expectedHash='model-sha')])) )
            def run(arguments, **kwargs):
                if 'input' not in kwargs:
                    kwargs['stdout'].write(b'{"ok":true}')
                    return subprocess.CompletedProcess(arguments, 0)
                requests = [json.loads(row) for row in kwargs['input'].splitlines()]
                infer = [request for request in requests if request['op'] == 'infer']
                self.assertEqual([request['id'] for request in infer], [2, 3, 4])
                self.assertEqual(requests[0]['op'], 'init')
                self.assertEqual(requests[-1]['op'], 'shutdown')
                for request, expected in zip(infer, (True, False, True)):
                    (Path(request['directory']) / 'result.json').write_text(json.dumps({'ok': expected}))
                return subprocess.CompletedProcess(arguments, 0, stdout=b'\n'.join(
                    json.dumps({'id': request['id'], 'op': 'done'}).encode() for request in infer))
            with patch.object(native, 'prepare', return_value=(root / 'cli', model)), \
                    patch.object(native.subprocess, 'run', side_effect=run):
                report = native.check(choice, root, None, None, root / 'worker', False)
            self.assertEqual(report['engineVersion'], 'b11960')
            self.assertEqual(report['modelSha256'], 'model-sha')
            self.assertGreaterEqual(report['directSeconds'], 0)
            self.assertGreaterEqual(report['residentThreeRequestsSeconds'], 0)


class AcquisitionTimingTests(unittest.TestCase):
    def test_download_and_hash_measurements_preserve_the_budget_and_verified_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = b'synthetic model'
            digest = hashlib.sha256(data).hexdigest()
            fetch = assets.Fetch(100, base=root)
            with patch.object(assets.urllib.request, 'urlopen', return_value=io.BytesIO(data)):
                path = fetch.acquire('https://example.invalid/model', len(data), digest)
            self.assertEqual(path.read_bytes(), data)
            self.assertEqual(fetch.ledger['downloadedBytes'], len(data))
            self.assertEqual(fetch.ledger['attempts'][0]['state'], 'verified')
            self.assertGreater(fetch.timings['downloadSeconds'], 0)
            self.assertGreater(fetch.timings['verificationSeconds'], 0)
            downloaded = fetch.timings['downloadSeconds']
            with patch.object(assets.urllib.request, 'urlopen', side_effect=AssertionError('unexpected network')):
                self.assertEqual(fetch.acquire('https://example.invalid/model', len(data), digest), path)
            self.assertEqual(fetch.timings['downloadSeconds'], downloaded)

    def test_interrupted_download_retains_attempt_bytes_and_elapsed_time(self):
        with tempfile.TemporaryDirectory() as temporary:
            fetch = assets.Fetch(100, base=Path(temporary))
            response = Mock()
            response.__enter__ = Mock(return_value=response)
            response.__exit__ = Mock(return_value=False)
            response.read.side_effect = [b'part', OSError('connection lost')]
            with patch.object(assets.urllib.request, 'urlopen', return_value=response), \
                    self.assertRaisesRegex(OSError, 'connection lost'):
                fetch.acquire('https://example.invalid/model', 20, '0' * 64)
            self.assertEqual(fetch.ledger['downloadedBytes'], 4)
            self.assertEqual(fetch.ledger['attempts'][0]['state'], 'failed')
            self.assertGreater(fetch.timings['downloadSeconds'], 0)


class NativeArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.archive = self.root / 'runtime.tar'
        self.weights = self.root / 'weights.gguf'
        self.weights.write_bytes(b'synthetic model')
        self.payloads = {'bin/program': b'program bytes', 'lib/runtime.dll': b'runtime bytes'}
        self.make_archive()
        self.destination = self.root / 'extracted'
        self.destination.mkdir()

    def make_archive(self, link=False):
        with tarfile.open(self.archive, 'w') as archive:
            for index, (name, payload) in enumerate(self.payloads.items()):
                member = tarfile.TarInfo(name)
                if link and index == 0:
                    member.type = tarfile.SYMTYPE
                    member.linkname = '../outside'
                else:
                    member.size = len(payload)
                archive.addfile(member, None if member.issym() else io.BytesIO(payload))

    def choice(self):
        archive = dict(expectedSize=self.archive.stat().st_size,
                       expectedHash=hashlib.sha256(self.archive.read_bytes()).hexdigest())
        files = []
        sources = []
        for index, (name, payload) in enumerate(self.payloads.items()):
            files.append(dict(role='executable' if index == 0 else 'runtime_library',
                              relativePath=name, expectedSize=len(payload),
                              expectedHash=hashlib.sha256(payload).hexdigest()))
            sources.append(dict(url='https://example.invalid/runtime.tar',
                                archive={**archive, 'memberPath': name}))
        files.append(dict(role='llm', expectedSize=self.weights.stat().st_size,
                          expectedHash=hashlib.sha256(self.weights.read_bytes()).hexdigest()))
        sources.append(dict(url='https://example.invalid/model.gguf'))
        return dict(id='llama-b11533-qwen3-4b-q4km', recipe=dict(
            manifest=dict(engineVersion='b11960', files=files), sources=sources))

    def fetch(self):
        return Mock(timings={}, acquire=Mock(side_effect=lambda url, size, sha:
                    self.archive if url.endswith('.tar') else self.weights))

    def test_shared_archive_is_acquired_opened_and_checked_once_per_prepare(self):
        fetch = self.fetch()
        digest = native.hashlib.file_digest
        with patch.object(native.hashlib, 'file_digest', wraps=digest) as hashes, \
                patch.object(native.tarfile, 'open', wraps=tarfile.open) as opens:
            executable, model = native.prepare(self.choice(), self.destination, fetch, None)
        self.assertEqual(executable.read_bytes(), self.payloads['bin/program'])
        self.assertEqual(model, self.weights)
        self.assertEqual(opens.call_count, 1)
        self.assertEqual(hashes.call_count, 3)  # archive plus both extracted members
        self.assertEqual(fetch.acquire.call_count, 2)  # archive plus model
        again = self.root / 'again'
        again.mkdir()
        native.prepare(self.choice(), again, fetch, None)
        self.assertEqual(fetch.acquire.call_count, 4)

    def test_changed_expected_hash_is_not_reused_for_the_same_url(self):
        choice = self.choice()
        choice['recipe']['sources'][1]['archive']['expectedHash'] = '0' * 64
        fetch = self.fetch()
        with self.assertRaisesRegex(ValueError, 'SHA'):
            native.prepare(choice, self.destination, fetch, None)
        self.assertEqual(fetch.acquire.call_count, 2)

    def test_archive_and_member_corruption_fail(self):
        for corrupt_archive in (False, True):
            choice = self.choice()
            if corrupt_archive:
                for source in choice['recipe']['sources'][:2]:
                    source['archive']['expectedHash'] = '0' * 64
            else:
                choice['recipe']['manifest']['files'][0]['expectedHash'] = '0' * 64
            with self.subTest(archive=corrupt_archive), self.assertRaisesRegex(ValueError, 'SHA'):
                native.prepare(choice, self.destination, self.fetch(), None)

    def test_missing_member_and_correctly_hashed_invalid_tar_report_extraction_failure(self):
        for invalid_tar in (False, True):
            self.make_archive()
            if invalid_tar:
                self.archive.write_bytes(b'not an archive')
            choice = self.choice()
            choice['recipe']['sources'][0]['archive']['memberPath'] = 'absent'
            metrics = {}
            with self.subTest(invalid_tar=invalid_tar), self.assertRaises((KeyError, tarfile.ReadError)):
                native.prepare(choice, self.destination, self.fetch(), None, metrics)
            self.assertEqual(metrics['stage'], 'extraction')
            self.assertGreaterEqual(metrics['phaseSeconds']['extraction'], 0)

    def test_links_and_unsafe_destinations_fail(self):
        for name in ('../outside', '/outside', 'C:/outside', 'bin\\outside', 'bin/./outside'):
            choice = self.choice()
            choice['recipe']['manifest']['files'][0]['relativePath'] = name
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'destination'):
                native.prepare(choice, self.destination, self.fetch(), None)
        self.make_archive(link=True)
        with self.assertRaisesRegex(ValueError, 'member shape'):
            native.prepare(self.choice(), self.destination, self.fetch(), None)

    def test_in_place_archive_mutation_is_detected(self):
        copy = native.shutil.copyfileobj
        def changed(source, destination):
            copy(source, destination)
            with self.archive.open('ab') as archive:
                archive.write(b'changed')
        with patch.object(native.shutil, 'copyfileobj', side_effect=changed), \
                self.assertRaisesRegex(ValueError, 'changed during extraction'):
            native.prepare(self.choice(), self.destination, self.fetch(), None)

    def test_windows_creation_and_descriptor_change_times_can_differ(self):
        opened = SimpleNamespace(st_dev=1, st_ino=2, st_size=self.archive.stat().st_size,
                                 st_mtime_ns=20, st_ctime_ns=30, st_birthtime_ns=10,
                                 st_mode=stat.S_IFREG | 0o600)
        current = SimpleNamespace(**{**vars(opened), 'st_ctime_ns': 10})
        fake_os = SimpleNamespace(name='nt', fstat=Mock(return_value=opened))
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        with patch.object(native, 'os', fake_os), patch.object(Path, 'lstat', return_value=current):
            self.assertEqual(native.verify(self.archive, opened.st_size, digest), self.archive)
            with native.checked_archive(self.archive, opened.st_size, digest, None) as archive:
                self.assertEqual(set(archive.getnames()), set(self.payloads))
        changed = SimpleNamespace(**{**vars(opened), 'st_ctime_ns': 31})
        fake_os.fstat.side_effect = [opened, changed]
        with patch.object(native, 'os', fake_os), patch.object(Path, 'lstat', return_value=current), \
                self.assertRaisesRegex(ValueError, 'changed during verification'):
            native.verify(self.archive, opened.st_size, digest)


class NativeFailureReportTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.report = self.root / 'output/report.json'
        package = self.root / ('.cache/windows-runtime/package/workers'
                               if native.os.name == 'nt' else '.cache/inference-runtime/package')
        package.mkdir(parents=True)
        worker = package / ('mon-llama-worker.exe' if native.os.name == 'nt' else 'mon-llama-worker')
        worker.write_bytes(b'worker')
        (package / 'runtime-index.json').write_text(json.dumps([dict(
            engine='llama', bytes=6, sha256=hashlib.sha256(b'worker').hexdigest())]))
        catalog = self.root / ('.cache/windows-runtime/package/catalog.json'
                               if native.os.name == 'nt' else 'app/model-catalog/macos.json')
        catalog.parent.mkdir(parents=True, exist_ok=True)
        catalog.write_text(json.dumps([
            dict(id=ident, recipe=dict(manifest=dict(engineVersion='b11960')))
            for ident in native.REPRESENTATIVES]))

    def invoke(self):
        with patch.object(native, 'ROOT', self.root):
            return native.main(['--report', str(self.report)])

    def test_startup_failure_is_recorded_before_the_model_loop(self):
        next(self.root.rglob('runtime-index.json')).unlink()
        with self.assertRaises(FileNotFoundError):
            self.invoke()
        record, = json.loads(self.report.read_text())
        self.assertIsNone(record['choice'])
        self.assertEqual(record['status'], 'failed')
        self.assertEqual(record['stage'], 'startup')
        self.assertEqual(record['exitCode'], 1)

    def test_each_failed_phase_preserves_prior_results_and_stops_later_models(self):
        for phase in ('acquisition', 'verification', 'extraction', 'direct', 'resident'):
            calls = []
            def check(choice, root, fetch, models, worker, metal, metrics):
                calls.append(root)
                if len(calls) == 1:
                    return dict(directSeconds=0.1, residentThreeRequestsSeconds=0.2)
                with native.measured(metrics, phase):
                    raise subprocess.TimeoutExpired('synthetic check', 1)
            with self.subTest(phase=phase), patch.object(native, 'check', side_effect=check), \
                    self.assertRaises(subprocess.TimeoutExpired):
                self.invoke()
            records = json.loads(self.report.read_text())
            self.assertEqual(len(records), 2)
            self.assertEqual(records[0]['status'], 'passed')
            self.assertEqual(records[0]['directSeconds'], 0.1)
            self.assertEqual(records[1]['status'], 'failed')
            self.assertEqual(records[1]['stage'], phase)
            self.assertGreaterEqual(records[1]['phaseSeconds'][phase], 0)
            self.assertGreaterEqual(records[1]['totalSeconds'], records[1]['phaseSeconds'][phase])
            self.assertTrue(all(not directory.exists() for directory in calls))

    def test_successful_representatives_keep_the_existing_report_fields(self):
        with patch.object(native, 'check', return_value=dict(directSeconds=1.0,
                residentThreeRequestsSeconds=2.0, directPeakRssBytes=None)) as check:
            self.assertEqual(self.invoke(), 0)
        records = json.loads(self.report.read_text())
        self.assertEqual(check.call_count, len(native.REPRESENTATIVES))
        self.assertEqual([record['choice'] for record in records], list(native.REPRESENTATIVES))
        self.assertTrue(all(record['status'] == 'passed' and record['exitCode'] == 0 for record in records))
        self.assertTrue(all(record['runTotalSeconds'] >= record['totalSeconds'] for record in records))


class RepresentativeAssetIsolationTests(unittest.TestCase):
    def test_private_verification_assets_do_not_enter_the_shared_ci_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);shared=root/'public-cache';private=root/'verification-only'
            data=b'synthetic weights';digest=hashlib.sha256(data).hexdigest()
            (private/'assets').mkdir(parents=True);(private/'assets'/digest).write_bytes(data)
            (private/'download-ledger.json').write_text(json.dumps(dict(downloadedBytes=7,attempts=[])))
            with patch.object(assets,'BASE',shared),patch.object(assets,'LEDGER',shared/'download-ledger.json'):
                fetch=assets.Fetch(8,base=private)
                self.assertEqual(fetch.acquire('https://example.invalid/model',len(data),digest),private/'assets'/digest)
                self.assertEqual(fetch.ledger['downloadedBytes'],7)
                self.assertFalse(shared.exists())
                with self.assertRaisesRegex(ValueError,'budget'):assets.Fetch(6,base=private)

    def test_native_validation_reports_only_json_and_keeps_weights_out_of_cache_paths(self):
        import yaml
        workflow=yaml.safe_load((ROOT/'.github/workflows/desktop-platform-build.yml').read_text())
        steps=workflow['jobs']['native']['steps']
        for step in steps:
            if 'actions/cache/' in step.get('uses','') or 'actions/upload-artifact' in step.get('uses',''):
                self.assertNotIn('.cache/llama-runtime-check',step.get('with',{}).get('path',''))
        check=next(step for step in steps if step.get('name')=='Validate upgraded LLM runtimes with representative weights')
        self.assertEqual(check['if'],"env.MON_CI_FULL == 'true'")
        self.assertIn('--report output/rust-tests/llama-runtime-',check['run'])
