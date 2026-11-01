"""Offline payload identity and signed metadata checks; no certificate or network IO."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("msix_local_test", ROOT / "tools/mon_windows_msix_test.py")
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


def digest(data):
    return hashlib.sha256(data).hexdigest()


class LocalStorePayloadTests(unittest.TestCase):
    def setUp(self):
        output = ROOT / "output"
        output.mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="msix-local-test-unit-", dir=output)
        if not Path(self.temp.name).resolve().is_relative_to(output.resolve()):
            raise AssertionError("Test cleanup must stay inside workspace output")
        self.addCleanup(self.temp.cleanup)
        self.package = Path(self.temp.name) / "local-test.msix"
        self.payload = {"AppxManifest.xml": b'<Identity Version="1.0.1.0"/>', "mon-desktop.exe": b"MZsame-original-executable"}
        self.files = [{"path": name, "bytes": len(value), "sha256": digest(value)} for name, value in self.payload.items()]
        self.contents = {**self.payload, "AppxBlockMap.xml": b"unchanged block map", "[Content_Types].xml": b"signed content types",
                         "AppxSignature.p7x": b"PKCXcms-is-verified-by-Windows-script", "AppxMetadata/CodeIntegrity.cat": b"\x30catalog"}
        self.metadata = {tag: digest(self.contents[name]) for tag, name in helper.METADATA_TAGS.items()}
        self.blockmap = digest(self.contents["AppxBlockMap.xml"])

    def write(self, extra=None):
        with zipfile.ZipFile(self.package, "w") as archive:
            for name, data in self.contents.items():
                archive.writestr(name, data)
            for name, data in (extra or {}).items():
                archive.writestr(name, data)

    def verify(self):
        return helper.verify_signed_payload(self.package, self.files, self.metadata, self.blockmap)

    def test_same_version_and_every_payload_byte_are_preserved(self):
        self.write()
        self.assertEqual(self.verify(), 2)

    def test_any_manifest_or_executable_change_is_rejected(self):
        for name in self.payload:
            with self.subTest(file=name):
                original = self.contents[name]
                self.contents[name] = original[:-1] + b"X"
                self.write()
                with self.assertRaisesRegex(ValueError, "payload hash mismatch"):
                    self.verify()
                self.contents[name] = original

    def test_unexpected_or_missing_or_case_aliased_files_are_rejected(self):
        for extra in ({"extra.exe": b"MZ"}, {"MON-DESKTOP.EXE": b"MZ"}):
            self.write(extra)
            with self.assertRaisesRegex(ValueError, "inventory"):
                self.verify()
        self.contents.pop("AppxMetadata/CodeIntegrity.cat")
        self.write()
        with self.assertRaisesRegex(ValueError, "inventory"):
            self.verify()

    def test_catalog_and_content_types_must_match_signed_cms_hashes(self):
        for name in ("AppxMetadata/CodeIntegrity.cat", "[Content_Types].xml"):
            with self.subTest(file=name):
                original = self.contents[name]
                self.contents[name] += b"changed"
                self.write()
                with self.assertRaisesRegex(ValueError, "CMS-signed metadata"):
                    self.verify()
                self.contents[name] = original

    def test_even_cms_matched_blockmap_must_remain_identical_to_unsigned_original(self):
        self.contents["AppxBlockMap.xml"] = b"changed map"
        self.metadata["AXBM"] = digest(self.contents["AppxBlockMap.xml"])
        self.write()
        with self.assertRaisesRegex(ValueError, "original block map"):
            self.verify()

    def test_bad_signature_or_missing_metadata_evidence_is_rejected(self):
        self.contents["AppxSignature.p7x"] = b"bad signature header"
        self.write()
        with self.assertRaisesRegex(ValueError, "signature header"):
            self.verify()
        self.contents["AppxSignature.p7x"] = b"PKCXstub"
        self.write()
        self.metadata.pop("AXCI")
        with self.assertRaisesRegex(ValueError, "metadata hash evidence"):
            self.verify()

    def test_output_parent_traversal_is_rejected_before_writing(self):
        with self.assertRaisesRegex(ValueError, "parent directories"):
            helper.output_path(ROOT / "output/../../escape")


if __name__ == "__main__":
    unittest.main()
