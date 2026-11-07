"""Offline payload identity and signed metadata checks; no certificate or network IO."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
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

    def finish_inputs(self, workspace=ROOT):
        """Use real ZIP/hash checks with a stub for the previously verified build."""
        self.write()
        directory = Path(self.temp.name)
        original = directory / "unsigned.msix"
        original.write_bytes(b"verified unsigned original")
        certificate = directory / "test-signing-public.cer"
        certificate.write_bytes(b"public certificate fixture")
        identity = {"displayName": "MinutesOn", "version": "1.0.1.0", "architecture": "x64", "publisher": "CN=fixture"}
        build = {"identity": identity, "files": self.files}
        manifest_path = directory / "build.json"
        manifest_path.write_text(json.dumps(build), encoding="utf-8")
        plan = {
            "buildWorkspace": str(workspace), "buildManifest": str(manifest_path), "expectedCommit": "a" * 40,
            "buildManifestSha256": helper.sha(manifest_path), "originalPackage": str(original),
            "originalSha256": helper.sha(original), "testPackage": str(self.package),
            "identity": identity, "files": self.files, "originalBlockMapSha256": self.blockmap,
            "purpose": "payload-identical-local-store-test",
        }
        plan_path = directory / "test-plan.json"
        helper.save_new(plan_path, plan)
        report = {
            "signatureCryptoVerified": True, "windowsTrustVerified": False, "metadataHashes": self.metadata,
            "certificate": {
                "subject": identity["publisher"], "thumbprint": "B" * 40, "persistentKeyRetained": True,
                "nonExportable": True, "privateKeyExported": False, "publicCertificate": str(certificate),
                "publicCertificateSha256": helper.sha(certificate), "notAfter": "2027-02-05T10:33:01Z",
            },
        }
        report_path = directory / "signature-verification.json"
        helper.save_new(report_path, report)
        msix = SimpleNamespace(regular=lambda root, path: Path(path), evidence=SimpleNamespace(utc_now=lambda: "2026-11-07T10:33:01Z"))
        args = SimpleNamespace(plan=plan_path, signature_report=report_path)
        frozen_build = (msix, workspace, manifest_path, build, original)
        return args, frozen_build, plan, report

    def test_successful_finish_writes_artifact_specific_markdown_procedure(self):
        args, frozen_build, plan, signature_report = self.finish_inputs()
        signed_hash = helper.sha(self.package)
        with patch.object(helper, "verified_build", return_value=frozen_build):
            helper.finish(args)
        guide_path = Path(self.temp.name) / "検証手順.md"
        guide = guide_path.read_text(encoding="utf-8")
        self.assertFalse(guide_path.with_suffix(".txt").exists())
        for value in (plan["expectedCommit"], plan["identity"]["version"], str(self.package.parent), signed_hash,
                      signature_report["certificate"]["thumbprint"], signature_report["certificate"]["notAfter"],
                      signature_report["certificate"]["publicCertificateSha256"]):
            self.assertIn(value, guide)
        self.assertIn(f"$testDirectory = '{self.package.parent}'", guide)
        self.assertIn("$expectedThumbprint = '" + "B" * 40 + "'", guide)
        self.assertNotIn(helper.GUIDE_SAMPLE_DIRECTORY, guide)
        self.assertNotIn(helper.GUIDE_SAMPLE_THUMBPRINT, guide)
        self.assertFalse(guide.startswith("---"))
        self.assertIn("```powershell\n", guide)
        self.assertIn("Add-AppxPackage -Path $package -ErrorAction Stop", guide)
        self.assertIn("Remove-AppxPackage -Package $testPackage[0].PackageFullName -Confirm", guide)
        self.assertIn("https://github.com/s-kodani/minuteson/blob/" + "a" * 40 + "/docs/guides/usage.md", guide)
        self.assertIn("https://learn.microsoft.com/en-us/powershell/module/appx/remove-appxpackage", guide)
        self.assertEqual(helper.sha(self.package), signed_hash)
        result = helper.read_json(self.package.parent / "test-manifest.json")
        self.assertFalse(result["packageInstalled"])
        self.assertFalse(result["certificateTrustImported"])

    def test_missing_or_invalid_canonical_guide_cannot_finish_preparation(self):
        workspace = Path(self.temp.name) / "source-workspace"
        args, frozen_build, _, _ = self.finish_inputs(workspace)
        guide_path = workspace / "docs/guides/testing-windows-msix.md"
        for contents in (None, "# Invalid guide without the required fields\n"):
            with self.subTest(contents=contents):
                if contents is not None:
                    guide_path.parent.mkdir(parents=True)
                    guide_path.write_text(contents, encoding="utf-8")
                with patch.object(helper, "verified_build", return_value=frozen_build):
                    with self.assertRaisesRegex(ValueError, "Canonical MSIX validation guide"):
                        helper.finish(args)
                self.assertFalse((self.package.parent / "test-manifest.json").exists())
                self.assertFalse((self.package.parent / "検証手順.md").exists())

    def test_existing_guide_is_preserved_without_a_new_completion_manifest(self):
        args, frozen_build, _, _ = self.finish_inputs()
        guide_path = self.package.parent / "検証手順.md"
        guide_path.write_text("# Existing validation guide\n", encoding="utf-8")
        with patch.object(helper, "verified_build", return_value=frozen_build):
            with self.assertRaises(FileExistsError):
                helper.finish(args)
        self.assertEqual(guide_path.read_text(encoding="utf-8"), "# Existing validation guide\n")
        self.assertFalse((self.package.parent / "test-manifest.json").exists())


if __name__ == "__main__":
    unittest.main()
