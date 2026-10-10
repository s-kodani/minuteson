import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "notices", Path(__file__).resolve().parents[1] / "tools/mon_third_party_notices.py")
notices = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(notices)


class NoticeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.file("app/src-tauri/Cargo.lock", "cargo lock")
        self.file("app/package-lock.json", "npm lock")
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
        path.write_text(text, encoding="utf8")
        return path

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
        (base / "LICENSE").symlink_to(outside)
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

    def test_generated_pair_can_be_read_and_linked_output_does_not_change_target(self):
        text, inventory = notices.collect(self.root, self.metadata, self.lock)
        output = self.root / "generated"
        notices.write_outputs(output, text, inventory)
        self.assertEqual((output / notices.OUTPUTS[0]).read_bytes(), text)
        self.assertEqual((output / notices.OUTPUTS[0]).stat().st_mode & 0o777, 0o644)
        self.assertEqual(json.loads((output / notices.OUTPUTS[1]).read_text()), inventory)
        target = self.file("outside.txt", "preserve")
        (output / notices.OUTPUTS[0]).unlink()
        (output / notices.OUTPUTS[0]).symlink_to(target)
        with self.assertRaises(ValueError):
            notices.write_outputs(output, text, inventory)
        self.assertEqual(target.read_text(), "preserve")


if __name__ == "__main__":
    unittest.main()
