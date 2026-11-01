"""Exercise publication without building or launching an installer."""
import hashlib
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'tools/publish-windows-installer.ps1'


@unittest.skipUnless(os.name == 'nt', 'Windows PowerShell publication')
class InstallerPublicationTests(unittest.TestCase):
    def setUp(self):
        (ROOT / 'output').mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='installer-publication-test-', dir=ROOT / 'output')
        self.root = Path(self.temp.name).resolve()
        self.assertTrue(self.root.is_relative_to(ROOT / 'output'))
        self.addCleanup(self.temp.cleanup)
        self.download = self.root / 'windows-qa'
        self.download.mkdir()
        self.source = self.root / 'built-setup.exe'
        self.source.write_bytes(b'MZ' + b'first installer payload' * 20)

    def publish(self, *, source=None, output=None, success=True, manifest=None, flavor=None):
        command = [
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(SCRIPT),
            '-Installer', str(source or self.source), '-OutputRoot', str(output or self.root),
        ]
        if manifest:
            command += ['-BuildManifest', str(manifest)]
        if flavor:
            command += ['-Flavor', flavor]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', errors='replace')
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        return result

    def test_release_uses_formal_name_and_preserves_qa_installer(self):
        self.publish()
        release = self.root / 'windows-release'
        release.mkdir()
        (release / 'MON-setup.exe').write_bytes(b'old release')
        (release / 'MinutesOn_1.0.0_x64-setup.exe').write_bytes(b'old versioned release')
        self.publish(flavor='release')
        self.assertEqual([p.name for p in release.iterdir()], ['MinutesOn-setup.exe'])
        self.assertEqual((release / 'MinutesOn-setup.exe').read_bytes(), self.source.read_bytes())
        self.assertEqual((self.download / 'MON-QA-setup.exe').read_bytes(), self.source.read_bytes())

    def test_repeated_publication_leaves_only_latest_and_preserves_diagnostics(self):
        (self.download / 'MON-QA-old-setup.exe').write_bytes(b'old')
        (self.download / 'build.log').write_text('evidence')
        (self.download / 'env.ps1').write_text('tools')
        (self.download / 'unfamiliar.exe').write_bytes(b'preserve')
        nested = self.download / 'screenshots'
        nested.mkdir()
        (nested / 'ui.png').write_bytes(b'image')
        self.publish()
        work = self.root / 'windows-qa-work'
        self.assertEqual((work / 'build.log').read_text(), 'evidence')
        self.assertEqual((work / 'env.ps1').read_text(), 'tools')
        self.assertEqual((work / 'unfamiliar.exe').read_bytes(), b'preserve')
        self.assertEqual((work / 'screenshots/ui.png').read_bytes(), b'image')
        self.assertFalse((work / 'MON-QA-old-setup.exe').exists())
        # A subsequent build changes the payload and accumulates no sidecars.
        self.source.write_bytes(b'MZ' + b'second installer payload' * 30)
        (self.download / 'build.log').write_text('new evidence')
        self.publish()
        files = list(self.download.iterdir())
        self.assertEqual([p.name for p in files], ['MON-QA-setup.exe'])
        self.assertEqual(hashlib.sha256(files[0].read_bytes()).digest(), hashlib.sha256(self.source.read_bytes()).digest())
        self.assertEqual((work / 'build.log').read_text(), 'evidence')
        self.assertEqual(len(list(work.glob('build.log.*'))), 1)
        self.assertEqual(list(work.glob('.installer-*.tmp')), [])

    def test_invalid_source_retains_previous_download_and_evidence(self):
        self.publish()
        previous = (self.download / 'MON-QA-setup.exe').read_bytes()
        (self.download / 'build.log').write_text('evidence')
        self.source.write_bytes(b'not an executable')
        self.publish(success=False)
        self.assertEqual((self.download / 'MON-QA-setup.exe').read_bytes(), previous)
        self.assertTrue((self.download / 'build.log').exists())

    def test_manifest_mismatch_retains_previous_download_and_evidence(self):
        self.publish()
        previous = (self.download / 'MON-QA-setup.exe').read_bytes()
        (self.download / 'build.log').write_text('preserve evidence')
        helper = self.root / 'capture.exe'
        helper.write_bytes(b'MZcapture helper')
        manifest = self.root / 'build.json'
        snapshot = self.root / 'source.json'
        evidence_tool = ROOT / 'tools/mon_windows_build_evidence.py'
        for arguments in [
            ['begin', '--output', str(snapshot)],
            ['finish', '--snapshot', str(snapshot), '--app', str(self.source),
             '--helper', str(helper), '--installer', str(self.source), '--output', str(manifest)],
        ]:
            subprocess.run(['python', str(evidence_tool), *arguments], check=True, capture_output=True)
        self.publish(manifest=manifest)
        # Source remains a valid PE-looking file, but its recorded helper no
        # longer matches. Publication must stop before moving diagnostics.
        (self.download / 'build.log').write_text('preserve evidence')
        helper.write_bytes(b'MZchanged helper')
        self.publish(manifest=manifest, success=False)
        self.assertEqual((self.download / 'MON-QA-setup.exe').read_bytes(), previous)
        self.assertEqual((self.download / 'build.log').read_text(), 'preserve evidence')

    def test_default_destination_is_the_workspace_output_directory(self):
        tools = self.root / 'tools'
        tools.mkdir()
        isolated = tools / SCRIPT.name
        shutil.copyfile(SCRIPT, isolated)
        result = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(isolated), '-Installer', str(self.source)], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / 'output/windows-qa/MON-QA-setup.exe').read_bytes(), self.source.read_bytes())

    def test_outside_workspace_is_rejected_before_mutation(self):
        outside = ROOT.parent / 'mon-installer-publication-must-not-create'
        self.assertFalse(outside.exists())
        self.publish(output=outside, success=False)
        self.assertFalse(outside.exists())
        self.assertEqual(list(self.download.iterdir()), [])

    def test_directory_junction_is_rejected_without_touching_target(self):
        target = self.root / 'must-preserve'
        target.mkdir()
        (target / 'data.txt').write_text('preserve')
        junction = self.download / 'linked'
        script = self.root / 'create-junction.ps1'
        script.write_text("param([string]$Path, [string]$Target)\n$ErrorActionPreference = 'Stop'\nNew-Item -ItemType Junction -Path $Path -Target $Target | Out-Null")
        # Pass paths as separate arguments rather than building shell commands.
        result = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(script), '-Path', str(junction), '-Target', str(target)], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        try:
            self.publish(success=False)
            self.assertEqual((target / 'data.txt').read_text(), 'preserve')
            self.assertTrue(junction.exists())
        finally:
            # Unlink the junction itself; never recurse into its target.
            if junction.exists():
                os.rmdir(junction)


if __name__ == '__main__':
    unittest.main()
