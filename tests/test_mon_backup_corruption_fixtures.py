import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("backup_fixtures", Path(__file__).resolve().parents[1] / "tools/mon_backup_corruption_fixtures.py")
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


def source_bytes():
    payload = b"QA-only payload"
    manifest = json.dumps({"entries": [{"size": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}]}).encode()
    return b"MONBAK01" + struct.pack("<I", len(manifest)) + hashlib.sha256(manifest).digest() + manifest + payload


class BackupCorruptionFixturesTests(unittest.TestCase):
    def test_four_copies_break_only_the_intended_boundary(self):
        body = source_bytes()
        copies = fixtures.corruption_copies(body)
        self.assertEqual(len(copies), 4)
        self.assertEqual(copies["wrong-magic.monbackup"][8:], body[8:])
        self.assertEqual(copies["truncated.monbackup"], body[:-1])
        self.assertEqual(copies["manifest-digest.monbackup"][:12], body[:12])
        self.assertEqual(copies["manifest-digest.monbackup"][13:], body[13:])
        self.assertEqual(copies["payload-digest.monbackup"][:-1], body[:-1])
        for copy in copies.values():
            self.assertNotEqual(copy, body)

    def test_reject_invalid_source_before_creating_outputs(self):
        for body in [b"MONBAK01", b"X" * 80, source_bytes()[:-1], source_bytes() + b"extra"]:
            with self.subTest(size=len(body)), self.assertRaises(ValueError):
                fixtures.corruption_copies(body)

    def test_reject_bad_manifest_and_payload_digests(self):
        body = source_bytes()
        copies = fixtures.corruption_copies(body)
        for name in ["manifest-digest.monbackup", "payload-digest.monbackup"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                fixtures.corruption_copies(copies[name])

    @unittest.skipUnless(hasattr(os, "O_NOFOLLOW"), "Private fixture generation requires POSIX no-follow opens and permissions")
    def test_generate_preserves_source_and_uses_private_non_overwriting_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.monbackup"
            body = source_bytes()
            source.write_bytes(body)
            before = source.stat()
            first = fixtures.generate(source, root)
            second = fixtures.generate(source, root)
            self.assertNotEqual(first["directory"], second["directory"])
            self.assertEqual(source.read_bytes(), body)
            self.assertEqual(source.stat().st_mtime_ns, before.st_mtime_ns)
            self.assertEqual(Path(first["directory"]).stat().st_mode & 0o777, 0o700)
            for entry in first["fixtures"]:
                self.assertEqual(Path(entry["path"]).stat().st_mode & 0o777, 0o600)
                self.assertEqual(entry["sha256"], hashlib.sha256(Path(entry["path"]).read_bytes()).hexdigest())

    @unittest.skipUnless(hasattr(os, "O_NOFOLLOW"), "Private fixture generation requires POSIX no-follow opens and permissions")
    def test_reject_source_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.monbackup"
            source.write_bytes(source_bytes())
            link = root / "link.monbackup"
            link.symlink_to(source)
            with self.assertRaises(OSError):
                fixtures.generate(link, root)


if __name__ == "__main__":
    unittest.main()
