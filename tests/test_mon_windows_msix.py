"""Store package identity, frozen payloads and failure-safe local publication."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock
import zipfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("mon_windows_msix", ROOT / "tools/mon_windows_msix.py")
msix = importlib.util.module_from_spec(spec)
spec.loader.exec_module(msix)


class StorePackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        for name in ("tauri.conf.json", "tauri.qa.conf.json", "tauri.windows.store.conf.json"):
            self.put("app/src-tauri/" + name, (ROOT / "app/src-tauri" / name).read_bytes())
        self.put("app/windows-store.json", (ROOT / "app/windows-store.json").read_bytes())
        self.config = msix.configuration(self.root)
        self.put("app/src-tauri/target/x86_64-pc-windows-msvc/release/mon-desktop.exe", b"MZapp")
        self.put("app/src-tauri/binaries/mon-windows-capture-x86_64-pc-windows-msvc.exe", b"MZcapture")
        for name in msix.RUNTIME_FILES:
            self.put(".cache/windows-runtime/package/" + name, b"runtime:" + name.encode())
        for name in msix.RUNTIME_DIRS:
            self.put(".cache/windows-runtime/package/" + name + "/worker.dll", b"MZlibrary")
        self.put(".cache/windows-runtime/package/asr/assets/models/bpe.vocab", b"vocabulary")
        self.put(".cache/windows-runtime/package/mon-speaker-embedding.pdb", b"debug only")
        self.put(".cache/windows-runtime/package/workers/worker.pdb", b"debug only")
        for name in msix.LOGOS:
            self.put("app/src-tauri/icons/" + name, (ROOT / "app/src-tauri/icons" / name).read_bytes())
        self.put(".cache/windows-capture-headers/LICENSE.MIT", b"license")
        self.put(".cache/windows-third-party-notices/THIRD_PARTY_NOTICES.txt", b"notices")
        self.put(".cache/windows-third-party-notices/third-party-inventory.json", b"{}")
        self.source = {"commit": "a" * 40, "patchSha256": "b" * 64,
                       "sourceTreeSha256": hashlib.sha256(b"[]").hexdigest(), "files": []}
        self.source_patch = mock.patch.object(msix.evidence, "source_identity", return_value=self.source)
        self.source_mock = self.source_patch.start()
        self.addCleanup(self.source_patch.stop)
        self.stage = self.root / "output/work/stage"
        self.package = self.root / "output/work/package.msix"
        self.manifest = self.root / "output/work/build.json"

    def put(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def package_stage(self, *, extra=None, omit=None, legacy=False, sdk=False):
        msix.stage(self.root, self.stage, self.config)
        if legacy:
            for name in msix.APP_ICON_LOGOS:
                (self.stage / "Assets" / name).unlink()
        elif sdk:
            msix.generate_resource_index(self.root, self.stage, self.config, self.root / "output/work/pri")
        else:
            # Unit fixtures freeze/hash this file; the SDK integration below
            # checks the actual index and every shell resource candidate.
            (self.stage / "resources.pri").write_bytes(b"unit PRI fixture")
        files = msix.inventory(self.root, self.stage)
        with zipfile.ZipFile(self.package, "w") as archive:
            for path in self.stage.rglob("*"):
                if path.is_file() and path.name != omit:
                    archive.write(path, path.relative_to(self.stage).as_posix())
            archive.writestr("AppxBlockMap.xml", '<BlockMap HashMethod="http://www.w3.org/2001/04/xmlenc#sha256"/>')
            archive.writestr("[Content_Types].xml", "<Types/>")
            if extra:
                archive.writestr(extra, b"unexpected")
        return files

    def finish(self):
        self.package_stage()
        snapshot = self.put("output/work/source.json", json.dumps({"schemaVersion": 1, "startedAt": "test", "source": self.source}).encode())
        return msix.finish(self.root, snapshot, self.stage, self.package, self.config, self.manifest)

    def test_store_identity_version_isolated_id_and_permissions(self):
        self.assertEqual(self.config["identityName"], "s-kodani.MinutesOn")
        self.assertEqual(self.config["version"], "1.0.0.0")
        self.assertEqual(self.config["tauriIdentifier"], "jp.mon.desktop.store")
        xml = msix.ET.fromstring(msix.manifest_xml(self.config))
        identity = xml.find("{" + msix.FOUNDATION + "}Identity")
        self.assertEqual(identity.get("Publisher"), "CN=C35CA2F5-F526-48AA-A0B4-26E70B666158")
        self.assertIn(b"runFullTrust", msix.manifest_xml(self.config))
        self.assertIn(b"microphone", msix.manifest_xml(self.config))
        self.assertIn(b"packagedClassicApp", msix.manifest_xml(self.config))

    def test_missing_identity_and_invalid_version_fail_without_output(self):
        store = self.root / "app/windows-store.json"
        original = store.read_bytes()
        for field in ("identityName", "publisher", "publisherDisplayName"):
            value = json.loads(original)
            value[field] = ""
            store.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                msix.configuration(self.root)
        store.write_bytes(original)
        base = self.root / "app/src-tauri/tauri.conf.json"
        for version in ("0.1.0", "1.0.0-beta", "1.0.65536", "1.0.0.1"):
            value = json.loads(base.read_text())
            value["version"] = version
            base.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                msix.configuration(self.root)
        self.assertFalse((self.root / "output").exists())

    def test_stage_keeps_vocabulary_dlls_notices_and_excludes_debug_files(self):
        files = self.package_stage()
        names = {f["path"] for f in files}
        self.assertIn("windows-runtime/asr/assets/models/bpe.vocab", names)
        self.assertIn("windows-runtime/llama/worker.dll", names)
        self.assertIn("notices/nlohmann-json-MIT.txt", names)
        self.assertIn("resources.pri", names)
        self.assertEqual(len(msix.APP_ICON_LOGOS), 42)
        self.assertTrue({"Assets/" + name for name in msix.APP_ICON_LOGOS} <= names)
        self.assertFalse(any(n.endswith(".pdb") for n in names))
        msix.verify_payload(self.package, self.config, files, msix.PAYLOAD_VERSION)

    def test_current_payload_rejects_missing_pri_or_theme_variant(self):
        files = self.package_stage()
        for name in ("resources.pri", "Assets/Square44x44Logo.targetsize-24_altform-lightunplated.png"):
            with self.subTest(missing=name):
                incomplete = [entry for entry in files if entry["path"] != name]
                with self.assertRaisesRegex(ValueError, "Required MSIX payload is missing"):
                    msix.verify_payload(self.package, self.config, incomplete, msix.PAYLOAD_VERSION)

    def test_missing_input_fails_before_creating_stage(self):
        (self.root / ".cache/windows-runtime/package/runtime.tar").unlink()
        with self.assertRaises(ValueError):
            msix.stage(self.root, self.stage, self.config)
        self.assertFalse(self.stage.exists())

    def test_archive_unexpected_or_missing_payload_is_rejected(self):
        files = self.package_stage(extra="unwanted.exe")
        with self.assertRaisesRegex(ValueError, "contents differ"):
            msix.verify_payload(self.package, self.config, files)
        self.package.unlink()
        shutil.rmtree(self.stage)
        files = self.package_stage(omit="mon-windows-capture.exe")
        with self.assertRaisesRegex(ValueError, "contents differ"):
            msix.verify_payload(self.package, self.config, files)

    def test_identity_and_payload_tampering_are_rejected(self):
        files = self.package_stage()
        changed = dict(self.config, publisher="CN=wrong")
        with self.assertRaisesRegex(ValueError, "identity or manifest"):
            msix.verify_payload(self.package, changed, files)
        files[0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            msix.verify_payload(self.package, self.config, files)

    def test_finish_and_verify_bind_source_and_package(self):
        result = self.finish()
        self.assertEqual(result["signing"], "microsoft-store-pending")
        self.assertEqual(result["payloadVersion"], 2)
        self.assertFalse(result["distributionReady"])
        self.assertTrue(msix.verify(self.root, self.manifest, require_current_source=True)["currentSourceMatches"])
        changed = copy.deepcopy(self.source)
        changed["commit"] = "c" * 40
        self.source_mock.return_value = changed
        with self.assertRaisesRegex(ValueError, "Current source differs"):
            msix.verify(self.root, self.manifest, require_current_source=True)

    def test_failed_publication_retains_previous_msix(self):
        self.finish()
        published = msix.publish(self.root, self.manifest)
        previous = published.read_bytes()
        self.package.write_bytes(b"broken package")
        with self.assertRaises(ValueError):
            msix.publish(self.root, self.manifest)
        self.assertEqual(published.read_bytes(), previous)

    def test_supplied_msix_can_be_verified_without_original_build_package(self):
        self.finish()
        published = msix.publish(self.root, self.manifest)
        self.package.unlink()
        result = msix.verify(self.root, self.manifest, package=published)
        self.assertTrue(result["currentSourceMatches"])
        self.assertEqual(result["packageSha256"], msix.evidence.sha256_file(published))

    def test_historical_evidence_without_qualified_icons_remains_verifiable(self):
        files = self.package_stage(legacy=True)
        legacy = {"schemaVersion": 1, "startedAt": "test", "completedAt": "test",
                  "source": self.source, "kind": "windows-store-msix", "identity": self.config,
                  "package": msix.artifact(self.root, self.package), "files": files}
        self.manifest.write_text(json.dumps(legacy), encoding="utf-8")
        self.assertTrue(msix.verify(self.root, self.manifest)["currentSourceMatches"])
        with self.assertRaisesRegex(ValueError, "Required MSIX payload is missing"):
            msix.verify_payload(self.package, self.config, files, msix.PAYLOAD_VERSION)
        legacy["payloadVersion"] = 999
        self.manifest.write_text(json.dumps(legacy), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "Unsupported MSIX payload version"):
            msix.verify(self.root, self.manifest)

    def test_copy_failure_preserves_previous_and_cleans_temporary(self):
        self.finish()
        published = msix.publish(self.root, self.manifest)
        previous = published.read_bytes()
        with mock.patch.object(msix.shutil, "copyfile", side_effect=lambda source, target: Path(target).write_bytes(b"bad copy")):
            with self.assertRaisesRegex(ValueError, "copy verification failed"):
                msix.publish(self.root, self.manifest)
        self.assertEqual(published.read_bytes(), previous)
        self.assertEqual(list(self.package.parent.glob(".publish-*")), [])

    def test_store_build_child_environment_disables_personal_signing(self):
        with mock.patch.dict(os.environ, {"MON_WINDOWS_RELEASE_SIGNING": "1", "TAURI_CONFIG": "qa", "CARGO_TARGET_DIR": "other"}):
            child = msix.build_environment(self.root)
            self.assertEqual(child["MON_WINDOWS_RELEASE_SIGNING"], "0")
            self.assertNotIn("TAURI_CONFIG", child)
            self.assertEqual(child["CARGO_TARGET_DIR"], str(self.root / "app/src-tauri/target"))
            self.assertEqual(os.environ["MON_WINDOWS_RELEASE_SIGNING"], "1")

    def test_staging_outside_workspace_is_rejected(self):
        with self.assertRaises(ValueError):
            msix.stage(self.root, self.root.parent / "must-not-stage", self.config)

    @unittest.skipUnless(os.name == "nt", "Windows build preflight")
    def test_missing_ctest_is_rejected_before_build_or_output_creation(self):
        with mock.patch.object(msix, "sdk_tool", return_value=self.root / "makeappx.exe"), \
                mock.patch.object(msix.shutil, "which", return_value=None), \
                mock.patch.object(msix.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "CMake/CTest is required"):
                msix.build(self.root)
        run.assert_not_called()
        self.assertFalse((self.root / "output").exists())

    @unittest.skipUnless(os.name == "nt", "Windows SDK package validation")
    def test_sdk_indexes_all_icon_variants_and_packs_the_manifest(self):
        try:
            tool = msix.sdk_tool("makeappx.exe")
            msix.sdk_tool("makepri.exe")
        except ValueError as error:
            self.skipTest(str(error))
        msix.stage(self.root, self.stage, self.config)
        work = self.root / "output/work/pri"
        msix.generate_resource_index(self.root, self.stage, self.config, work)
        dump = msix.ET.parse(work / "resources.pri.xml")
        square = next(r for r in dump.findall(".//NamedResource")
                      if r.get("uri") == "ms-resource://s-kodani.MinutesOn/Files/Assets/Square44x44Logo.png")
        for form in ("UNPLATED", "LIGHTUNPLATED"):
            candidates = [c for c in square.findall("Candidate")
                          if any(q.get("name") == "AlternateForm" and q.get("value") == form
                                 for q in c.findall("QualifierSet/Qualifier"))]
            self.assertEqual(len(candidates), 14)
            self.assertEqual({int(next(q.get("value") for q in c.findall("QualifierSet/Qualifier")
                                       if q.get("name") == "TargetSize")) for c in candidates},
                             {16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256})
        self.assertEqual({c.findtext("Value").replace("\\", "/") for c in dump.findall(".//Candidate")},
                         {"Assets/" + name for name in msix.LOGOS})
        pri_config = msix.ET.parse(work / "priconfig.xml")
        self.assertEqual(pri_config.find("index/default/qualifier[@name='Language']").get("value"), "ja-JP")
        self.assertFalse((self.stage / "priconfig.xml").exists())
        self.assertFalse((self.stage / "resources.pri.xml").exists())
        self.assertEqual({p.relative_to(work / "input").as_posix() for p in (work / "input").rglob("*") if p.is_file()},
                         {"Assets/" + name for name in msix.LOGOS})
        # A file named resources.pri is insufficient: incorrect logical paths
        # or missing theme candidates must fail the production dump validator.
        resource_map = dump.find("ResourceMap")
        resource_map.set("name", "wrong-package")
        changed = work / "changed.xml"
        dump.write(changed, encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "package identity"):
            msix.verify_resource_index(changed, self.config)
        resource_map.set("name", self.config["identityName"])
        original_uri = square.get("uri")
        square.set("uri", original_uri.replace("Files/Assets/", "Files/"))
        dump.write(changed, encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "manifest's Assets"):
            msix.verify_resource_index(changed, self.config)
        square.set("uri", original_uri)
        square.remove(candidates[0])
        dump.write(changed, encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "icon candidates"):
            msix.verify_resource_index(changed, self.config)
        files = msix.inventory(self.root, self.stage)
        result = subprocess.run([str(tool), "pack", "/d", str(self.stage), "/p", str(self.package), "/h", "SHA256", "/no"],
                                capture_output=True, text=True, errors="replace")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        msix.verify_payload(self.package, self.config, files, msix.PAYLOAD_VERSION)


if __name__ == "__main__":
    unittest.main()
