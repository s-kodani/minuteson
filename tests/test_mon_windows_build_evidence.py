"""Source drift and binary mismatch must not pass installer identification."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("build_evidence", ROOT / "tools/mon_windows_build_evidence.py")
evidence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evidence)


class BuildEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        (self.root / "app").mkdir()
        (self.root / "app/main.ts").write_text("original source\n", encoding="utf-8")
        self.git("init", "-q")
        self.git("add", "app")
        self.git("-c", "user.name=Build Test", "-c", "user.email=build@example.invalid", "commit", "-qm", "fixture")
        self.snapshot = self.root / "output/source.json"
        self.manifest = self.root / "output/build.json"
        self.app = self.root / "output/mon-desktop.exe"
        self.helper = self.root / "output/mon-windows-capture.exe"
        self.installer = self.root / "output/setup.exe"
        self.app.parent.mkdir()
        for path in (self.app, self.helper, self.installer):
            path.write_bytes(b"MZ" + path.name.encode())

    def git(self, *args):
        subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True)

    def finish(self):
        evidence.finish(self.root, self.snapshot, self.app, self.helper, self.installer, self.manifest)

    def test_dirty_source_is_identified_and_installed_binaries_are_compared(self):
        (self.root / "app/main.ts").write_text("modified source\n", encoding="utf-8")
        (self.root / "app/untracked.ts").write_text("new source\n", encoding="utf-8")
        evidence.begin(self.root, self.snapshot)
        self.finish()
        manifest = json.loads(self.manifest.read_text())
        self.assertEqual(manifest["source"]["commit"], self.git_commit())
        self.assertEqual(len(manifest["source"]["files"]), 2)
        self.assertEqual(manifest["installedVerification"], "pending")
        installed = self.root / "isolated-install"
        installed.mkdir()
        installed_app = installed / self.app.name
        installed_helper = installed / self.helper.name
        installed_app.write_bytes(self.app.read_bytes())
        installed_helper.write_bytes(self.helper.read_bytes())
        published = self.root / "output/MON-QA-setup.exe"
        published.write_bytes(self.installer.read_bytes())
        result = evidence.verify(self.root, self.manifest, installed_app, installed_helper, published)
        self.assertEqual(set(result["installed"]), {"app", "helper"})
        self.assertTrue(result["publishedInstallerVerified"])
        self.assertTrue(result["currentSourceMatches"])
        installed_app.write_bytes(b"MZold installation")
        with self.assertRaisesRegex(ValueError, "Installed app"):
            evidence.verify(self.root, self.manifest, installed_app, installed_helper)
        published.write_bytes(b"MZold installer")
        with self.assertRaisesRegex(ValueError, "Published installer"):
            evidence.verify(self.root, self.manifest, published_installer=published)

    def git_commit(self):
        return evidence.git(self.root, "rev-parse", "HEAD").decode().strip()

    def test_changes_during_build_reject_manifest_without_overwriting_evidence(self):
        evidence.begin(self.root, self.snapshot)
        original = self.snapshot.read_bytes()
        with self.assertRaises(FileExistsError):
            evidence.begin(self.root, self.snapshot)
        self.assertEqual(self.snapshot.read_bytes(), original)
        for change in ("tracked", "untracked", "deleted"):
            with self.subTest(change=change):
                path = self.root / ("app/new.ts" if change == "untracked" else "app/main.ts")
                if change == "deleted":
                    path.unlink()
                else:
                    path.write_text(change, encoding="utf-8")
                with self.assertRaisesRegex(ValueError, "source changed"):
                    self.finish()
                self.assertFalse(self.manifest.exists())
                if change == "untracked":
                    path.unlink()
                (self.root / "app/main.ts").write_text("original source\n", encoding="utf-8")

    def test_post_build_binary_change_is_rejected(self):
        evidence.begin(self.root, self.snapshot)
        self.finish()
        for path in (self.app, self.helper, self.installer):
            previous = path.read_bytes()
            path.write_bytes(previous + b"changed")
            with self.assertRaisesRegex(ValueError, "does not match"):
                evidence.verify(self.root, self.manifest)
            path.write_bytes(previous)

    def test_post_build_source_drift_is_explicit_and_cannot_be_published(self):
        evidence.begin(self.root, self.snapshot)
        self.finish()
        (self.root / "app/main.ts").write_text("not rebuilt\n", encoding="utf-8")
        self.assertFalse(evidence.verify(self.root, self.manifest)["currentSourceMatches"])
        with self.assertRaisesRegex(ValueError, "Current source"):
            evidence.verify(self.root, self.manifest, require_current_source=True)
        manifest = json.loads(self.manifest.read_text())
        del manifest["source"]
        self.manifest.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "source identity"):
            evidence.verify(self.root, self.manifest)

    def test_installed_check_does_not_require_the_original_build_directory(self):
        evidence.begin(self.root, self.snapshot)
        self.finish()
        copies = []
        for original in (self.app, self.helper, self.installer):
            copy = original.with_name("installed-" + original.name)
            copy.write_bytes(original.read_bytes())
            copies.append(copy)
            original.unlink()
        result = evidence.verify(self.root, self.manifest, *copies)
        self.assertEqual(set(result["installed"]), {"app", "helper"})
        self.assertTrue(result["publishedInstallerVerified"])
        with self.assertRaises(FileNotFoundError):
            evidence.verify(self.root, self.manifest, require_current_source=True)

    def test_non_executable_and_outside_evidence_are_rejected(self):
        evidence.begin(self.root, self.snapshot)
        self.helper.write_bytes(b"not an executable")
        with self.assertRaisesRegex(ValueError, "Windows executable"):
            self.finish()
        self.assertFalse(self.manifest.exists())
        with self.assertRaisesRegex(ValueError, "inside the workspace"):
            evidence.begin(self.root, self.root.parent / "must-not-write-build-evidence.json")


if __name__ == "__main__":
    unittest.main()
