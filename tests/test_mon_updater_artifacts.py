import base64
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import mon_updater_artifacts as updater

KEY = base64.b64encode(b"untrusted comment: test\n" + base64.b64encode(b"Ed" + bytes(40)) + b"\n").decode()


class UpdaterArtifacts(unittest.TestCase):
    def test_missing_and_malformed_keys_fail_closed(self):
        for key in ["", "bad", base64.b64encode(b"untrusted comment: key\nRWQ=\n").decode()]:
            with self.assertRaises(ValueError):
                updater.public_key({"TAURI_UPDATER_PUBLIC_KEY": key})
        config = json.loads(updater.configuration({"TAURI_UPDATER_PUBLIC_KEY": KEY}))
        self.assertEqual(config["plugins"]["updater"]["pubkey"], KEY)
        self.assertTrue(config["plugins"]["updater"]["requireSignedVersion"])

    def test_archive_preserves_bundle_root_permissions_and_internal_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            app = Path(temporary) / "MinutesOn.app"
            binary = app / "Contents/MacOS/mon-desktop"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"test executable")
            binary.chmod(0o755)
            (binary.parent / "helper").symlink_to("mon-desktop")
            def sign(argv, **kwargs):
                self.assertIn("--app-version", argv)
                Path(str(argv[-1]) + ".sig").write_text("signature")
            with patch.object(updater.subprocess, "run", side_effect=sign), patch.object(updater, "validate_bundle_archive"), patch.object(updater, "verify") as verify:
                assets = updater.create(app, "1.0.4", {"TAURI_UPDATER_PUBLIC_KEY": KEY, "TAURI_SIGNING_PRIVATE_KEY": "synthetic"})
            verify.assert_called_once()
            with tarfile.open(assets[0]) as archive:
                self.assertEqual(archive.getmember("MinutesOn.app/Contents/MacOS/mon-desktop").mode & 0o777, 0o755)
                self.assertEqual(archive.getmember("MinutesOn.app/Contents/MacOS/helper").linkname, "mon-desktop")
            self.assertEqual(updater.validate_metadata(app.parent, "1.0.4"), [a.name for a in assets])
            metadata = json.loads(assets[2].read_text())
            metadata["platforms"]["darwin-aarch64"]["url"] = "https://example.test/app"
            assets[2].write_text(json.dumps(metadata))
            with self.assertRaises(ValueError):
                updater.validate_metadata(app.parent, "1.0.4")

    @unittest.skipUnless(os.environ.get("MON_UPDATER_INTEGRATION") == "1", "requires built verifier and installed Tauri CLI")
    def test_real_signatures_reject_tampering_wrong_key_and_wrong_version(self):
        cli = updater.ROOT / "app/node_modules/.bin/tauri"
        with tempfile.TemporaryDirectory(prefix="mon-updater-test-") as temporary:
            root = Path(temporary)
            keys = []
            for name in ("first", "second"):
                key = root / name
                subprocess.run([str(cli), "signer", "generate", "--ci", "--password", "", "--write-keys", str(key)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                keys.append(key)
            app = root / "MinutesOn.app"
            app.mkdir()
            (app / "payload").write_bytes(b"synthetic update")
            with patch.object(updater, "validate_bundle_archive"):
                assets = updater.create(app, "1.0.4", {**os.environ, "TAURI_UPDATER_PUBLIC_KEY": keys[0].with_suffix(".pub").read_text().strip(), "TAURI_SIGNING_PRIVATE_KEY": keys[0].read_text().strip(), "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": ""})
            key = keys[0].with_suffix(".pub").read_text().strip()
            for public, version in [(key, "1.0.5"), (keys[1].with_suffix(".pub").read_text().strip(), "1.0.4")]:
                with self.assertRaises(subprocess.CalledProcessError):
                    updater.verify(assets[0], assets[1], public, version)
            assets[0].write_bytes(assets[0].read_bytes() + b"tampered")
            with self.assertRaises(subprocess.CalledProcessError):
                updater.verify(assets[0], assets[1], key, "1.0.4")
