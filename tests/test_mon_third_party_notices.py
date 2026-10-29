import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "notices", Path(__file__).resolve().parents[1] / "tools/mon_third_party_notices.py")
notices = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(notices)


class NoticeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.file("Cargo.lock", "version = 4\n")
        self.file("app/package-lock.json", "npm lock")
        self.file("third-party/notices/index.json", json.dumps({"schemaVersion": 1, "files": [], "packages": []}))
        self.file("crate/Cargo.toml", "manifest")
        self.file("crate/LICENSE", "Copyright synthetic\nPermission synthetic\n")
        self.file("crate/vendor/NOTICE", "Additional synthetic notice\n")
        self.file("app/node_modules/runtime/package.json", json.dumps({"name": "runtime", "version": "1.0"}))
        self.file("app/node_modules/runtime/LICENSE", "Npm synthetic notice\n")
        self.metadata = {"resolve": {"root": "root", "nodes": [
            {"id": "root", "deps": [{"pkg": "crate", "dep_kinds": [{"kind": None}]},
                                       {"pkg": "build", "dep_kinds": [{"kind": "build"}]},
                                       {"pkg": "dev", "dep_kinds": [{"kind": "dev"}]}]},
            {"id": "crate", "deps": []}, {"id": "build", "deps": []}, {"id": "dev", "deps": []}]},
            "packages": [{"id": key, "name": key, "version": "1.0", "license": "MIT",
                          "manifest_path": str(self.root / "crate/Cargo.toml")} for key in
                         ["root", "crate", "build", "dev", "unreachable-platform"]]}
        self.lock = {"lockfileVersion": 3, "packages": {
            "": {}, "node_modules/runtime": {"version": "1.0", "license": "MIT"},
            "node_modules/build-tool": {"version": "1.0", "dev": True}}}

    def file(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf8", newline="")
        return path

    def symlink(self, path, target):
        try:
            path.symlink_to(target)
        except OSError as error:
            if getattr(error, "winerror", None) == 1314:
                self.skipTest("Windows symbolic links require Developer Mode or the symlink privilege")
            raise

    def test_graph_excludes_root_dev_and_unreachable_platform_but_keeps_build(self):
        self.assertEqual([p["id"] for p in notices.cargo_packages(self.metadata)], ["build", "crate"])

    def test_verbatim_nested_notices_determinism_hashes_and_no_local_paths(self):
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        self.assertEqual(notices.collect(self.root, self.metadata, self.lock), (text, inventory))
        self.assertIn(b"Copyright synthetic\nPermission synthetic\n", text)
        self.assertIn(b"Additional synthetic notice\n", text)
        self.assertEqual(inventory["noticesSha256"], notices.sha(text))
        self.assertEqual(len(inventory["packages"]), 3)
        self.assertNotIn(str(self.root), json.dumps(inventory))
        self.assertNotIn("build-tool", text.decode())
        self.assertFalse(inventory["distributionReady"])
        self.assertEqual(inventory["licenseReview"], "not_verified")

    def test_notice_temporary_files_are_closed_before_publication(self):
        output = self.root / "generated"
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        streams = []
        temporary_file = notices.tempfile.NamedTemporaryFile
        replace = notices.os.replace

        def track_temporary(*args, **kwargs):
            stream = temporary_file(*args, **kwargs)
            streams.append(stream)
            return stream

        def publish(source, destination):
            self.assertTrue(streams[-1].closed)
            replace(source, destination)

        with patch.object(notices.tempfile, "NamedTemporaryFile", side_effect=track_temporary), \
                patch.object(notices.os, "replace", side_effect=publish):
            notices.write_outputs(output, text, inventory)
        self.assertEqual(len(streams), 2)
        self.assertEqual(list(output.glob(".notice-*")), [])

    def test_windows_inventory_identifies_the_target_and_system_exclusions(self):
        _, inventory = notices.collect(self.root, self.metadata, self.lock, "x86_64-pc-windows-msvc")
        self.assertEqual(inventory["target"], "x86_64-pc-windows-msvc")
        self.assertIn("Microsoft Windows SDK/system libraries", inventory["excluded"])
        self.assertNotIn("Apple SDK/system frameworks", inventory["excluded"])
        with self.assertRaises(ValueError):
            notices.collect(self.root, self.metadata, self.lock, "unreviewed-target")

    def test_missing_text_is_explicit_not_license_success_or_generic_replacement(self):
        (self.root / "crate/LICENSE").unlink()
        (self.root / "crate/vendor/NOTICE").unlink()
        _, inventory = notices.collect(self.root, self.metadata, self.lock)
        self.assertEqual(inventory["missingNoticeTexts"], ["cargo:build@1.0", "cargo:crate@1.0"])
        self.assertEqual(inventory["licenseReview"], "not_verified")

    def test_npm_missing_or_mismatched_install_refuses_instead_of_using_stale_notices(self):
        for value in [{"name": "runtime", "version": "2.0"}, {"name": "foreign", "version": "1.0"}, {}]:
            self.file("app/node_modules/runtime/package.json", json.dumps(value))
            with self.assertRaises((ValueError, KeyError)):
                notices.collect(self.root, self.metadata, self.lock)

    def test_scoped_and_nested_production_packages_use_their_own_notice_text(self):
        for location, name in [("node_modules/@example/scoped", "@example/scoped"),
                               ("node_modules/runtime/node_modules/nested", "nested")]:
            self.file(f"app/{location}/package.json", json.dumps({"name": name, "version": "1.0"}))
            self.file(f"app/{location}/LICENSE", f"Notice for {name}")
            self.lock["packages"][location] = {"version": "1.0", "license": "MIT"}
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        self.assertIn(b"Notice for @example/scoped", text)
        self.assertIn(b"Notice for nested", text)
        self.assertEqual(len(inventory["packages"]), 5)

    def test_linked_or_escaping_declared_notices_are_rejected(self):
        base = self.root / "crate"
        outside = self.file("outside/LICENSE", "outside")
        with self.assertRaises(ValueError):
            notices.notice_files(base, "../outside/LICENSE")
        (base / "LICENSE").unlink()
        self.symlink(base / "LICENSE", outside)
        with self.assertRaises(ValueError):
            notices.notice_files(base)

    def test_size_and_invalid_text_are_not_silently_omitted(self):
        path = self.root / "crate/LICENSE"
        path.write_bytes(b"\xff")
        with self.assertRaises(ValueError):
            notices.notice_files(self.root / "crate")
        path.write_bytes(b"x" * (notices.MAX_TEXT_BYTES + 1))
        with self.assertRaises(ValueError):
            notices.notice_files(self.root / "crate")

    def test_license_directory_texts_are_included_but_copying_source_is_not_a_notice(self):
        self.file("crate/LICENSES/MIT.txt", "Directory MIT text")
        self.file("crate/LICENSES/Apache-2.0.txt", "Directory Apache text")
        self.file("crate/LICENSE.MIT", "Named MIT text")
        self.file("crate/LICENSE.BSD-3-Clause", "Named BSD text")
        self.file("crate/src/copying.rs", "pub fn copying() {}")
        self.file("crate/vendor/LICENSE.py", "print('not a notice')")
        names = [Path(name).as_posix() for name, _ in notices.notice_files(self.root / "crate")]
        self.assertIn("LICENSES/MIT.txt", names)
        self.assertIn("LICENSES/Apache-2.0.txt", names)
        self.assertIn("LICENSE.MIT", names)
        self.assertIn("LICENSE.BSD-3-Clause", names)
        self.assertNotIn("src/copying.rs", names)
        self.assertNotIn("vendor/LICENSE.py", names)

    def test_generated_pair_can_be_read_and_linked_output_does_not_change_target(self):
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        output = self.root / "generated"
        notices.write_outputs(output, text, inventory)
        self.assertEqual((output / notices.OUTPUTS[0]).read_bytes(), text)
        if notices.os.name != "nt":
            self.assertEqual((output / notices.OUTPUTS[0]).stat().st_mode & 0o777, 0o644)
        self.assertEqual(json.loads((output / notices.OUTPUTS[1]).read_text(encoding="utf8")), inventory)
        target = self.file("outside.txt", "preserve")
        (output / notices.OUTPUTS[0]).unlink()
        self.symlink(output / notices.OUTPUTS[0], target)
        with self.assertRaises(ValueError):
            notices.write_outputs(output, text, inventory)
        self.assertEqual(target.read_text(), "preserve")

    def supplement(self, kind="license_text"):
        self.file("crate/.cargo_vcs_info.json", json.dumps({"git": {"sha1": "a" * 40}, "path_in_vcs": "crate"}))
        self.file("Cargo.lock", 'version = 4\n[[package]]\nname = "crate"\nversion = "1.0"\n'
                  f'source = "{notices.REGISTRY}"\nchecksum = "' + "1" * 64 + '"\n')
        package = next(p for p in self.metadata["packages"] if p["id"] == "crate")
        package["source"] = notices.REGISTRY
        data = b"Upstream synthetic license/reference\n"
        self.file("third-party/notices/upstream.txt", data.decode())
        index = {"schemaVersion": 1,
                 "files": [{"path": "upstream.txt", "kind": kind, "sourceUrl": "https://example.invalid/fixed/LICENSE",
                            "sha256": notices.sha(data), "bytes": len(data)}],
                 "packages": [{"name": "crate", "version": "1.0", "declaredLicense": "MIT",
                               "crateChecksum": "1" * 64, "vcsCommit": "a" * 40, "vcsPath": "crate",
                               "files": ["upstream.txt"]}]}
        self.file("third-party/notices/index.json", json.dumps(index))
        (self.root / "crate/LICENSE").unlink()
        (self.root / "crate/vendor/NOTICE").unlink()
        return index, package

    def test_exact_version_supplement_retains_origin_hash_and_source_reference_without_approval(self):
        self.supplement()
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        entry = next(p for p in inventory["packages"] if p["name"] == "crate")
        self.assertIn(b"Upstream synthetic license/reference\n", text)
        self.assertEqual(entry["supplementalFiles"][0]["kind"], "license_text")
        self.assertEqual(entry["sourceArchive"]["sha256"], "1" * 64)
        self.assertEqual(entry["sourceArchive"]["url"], "https://static.crates.io/crates/crate/crate-1.0.crate")
        self.assertNotIn("cargo:crate@1.0", inventory["missingNoticeTexts"])
        self.assertEqual(inventory["licenseReview"], "not_verified")

    def test_reference_only_description_does_not_resolve_missing_license_text(self):
        self.supplement("upstream_license_reference")
        _, inventory = notices.collect(self.root, self.metadata, self.lock)
        self.assertIn("cargo:crate@1.0", inventory["missingNoticeTexts"])
        self.assertEqual(inventory["referenceOnlyPackages"], ["cargo:crate@1.0"])

    def test_standard_template_is_readable_offline_without_resolving_attribution_or_approval(self):
        self.supplement("license_template")
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        self.assertIn(b"Upstream synthetic license/reference\n", text)
        self.assertIn(b"Standard license template only; NOT a package-specific copyright notice.", text)
        self.assertIn("cargo:crate@1.0", inventory["missingNoticeTexts"])
        self.assertEqual(inventory["referenceOnlyPackages"], ["cargo:crate@1.0"])
        entry = next(p for p in inventory["packages"] if p["name"] == "crate")
        self.assertEqual(entry["supplementalFiles"][0]["kind"], "license_template")
        self.assertEqual(inventory["licenseReview"], "not_verified")
        self.assertFalse(inventory["distributionReady"])

    def test_reference_and_template_together_still_leave_missing_package_notice(self):
        index, _ = self.supplement("upstream_license_reference")
        template = b"Copyright (c) <year> <copyright holders>\nSynthetic permission template\n"
        self.file("third-party/notices/template.txt", template.decode())
        index["files"].append({"path": "template.txt", "kind": "license_template",
                               "sourceUrl": "https://example.invalid/fixed/MIT.txt",
                               "sha256": notices.sha(template), "bytes": len(template)})
        index["packages"][0]["files"].append("template.txt")
        self.file("third-party/notices/index.json", json.dumps(index))
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        self.assertIn(template, text)
        self.assertIn("cargo:crate@1.0", inventory["missingNoticeTexts"])
        self.assertEqual(inventory["referenceOnlyPackages"], ["cargo:crate@1.0"])
        self.file("third-party/notices/template.txt", "modified")
        with self.assertRaises(ValueError):
            notices.collect(self.root, self.metadata, self.lock)

    def test_mpl_notice_informs_recipients_of_exact_source_without_automatic_fetch(self):
        index, package = self.supplement()
        package["license"] = "MPL-2.0"
        index["packages"][0]["declaredLicense"] = "MPL-2.0"
        self.file("third-party/notices/index.json", json.dumps(index))
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        self.assertIn(b"MPL source reference (retrieval not verified): https://static.crates.io/crates/crate/crate-1.0.crate", text)
        self.assertIn(("Expected source archive SHA-256: " + "1" * 64).encode(), text)
        entry = next(p for p in inventory["packages"] if p["name"] == "crate")
        self.assertEqual(entry["sourceArchive"]["state"], "source_reference_not_download_verified")
        self.assertFalse(inventory["distributionReady"])

    def test_supplement_license_vcs_archive_hash_or_content_changes_are_refused(self):
        original, package = self.supplement()
        for key, value in [("declaredLicense", "Apache-2.0"), ("vcsCommit", "b" * 40),
                           ("vcsPath", "other"), ("crateChecksum", "2" * 64)]:
            changed = json.loads(json.dumps(original))
            changed["packages"][0][key] = value
            self.file("third-party/notices/index.json", json.dumps(changed))
            with self.subTest(key=key), self.assertRaises(ValueError):
                notices.collect(self.root, self.metadata, self.lock)
        self.file("third-party/notices/index.json", json.dumps(original))
        self.file("third-party/notices/upstream.txt", "changed")
        with self.assertRaises(ValueError):
            notices.collect(self.root, self.metadata, self.lock)

    def test_missing_index_or_linked_supplement_and_foreign_source_are_refused(self):
        original, package = self.supplement()
        package["source"] = "git+https://example.invalid/foreign"
        with self.assertRaises(ValueError):
            notices.collect(self.root, self.metadata, self.lock)
        package["source"] = notices.REGISTRY
        vcs_path = self.root / "crate/.cargo_vcs_info.json"
        original_vcs = vcs_path.read_text(encoding="utf8")
        vcs_path.unlink()
        self.symlink(vcs_path, self.file("outside-vcs.json", original_vcs))
        with self.assertRaises(ValueError):
            notices.collect(self.root, self.metadata, self.lock)
        vcs_path.unlink()
        self.file("crate/.cargo_vcs_info.json", original_vcs)
        path = self.root / "third-party/notices/upstream.txt"
        path.unlink()
        self.symlink(path, self.file("outside.txt", "preserve"))
        with self.assertRaises(ValueError):
            notices.collect(self.root, self.metadata, self.lock)
        (self.root / "third-party/notices/index.json").unlink()
        with self.assertRaises(OSError):
            notices.collect(self.root, self.metadata, self.lock)


if __name__ == "__main__":
    unittest.main()
