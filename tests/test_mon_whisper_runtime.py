import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import sys
import unittest
from unittest.mock import patch

if sys.platform == "win32":
    raise unittest.SkipTest("The macOS Whisper runtime builder requires POSIX file locks and permissions")

SPEC = importlib.util.spec_from_file_location("mon_whisper_runtime", Path(__file__).resolve().parents[1] / "tools/mon_whisper_runtime.py")
runtime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


class RuntimeTests(unittest.TestCase):
    def test_native_fixed_build_definition_matches_builder_without_model_weights(self):
        definition = runtime.REPOSITORY / "app/model-catalog/whisper-runtime-build.json"
        self.assertEqual(json.loads(definition.read_text()), runtime.build_info())

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.binary = self.root / "whisper-cli"
        self.binary.write_bytes(b"synthetic Mach-O stand-in; never execute")
        self.binary.chmod(0o755)
        self.license = self.root / "LICENSE"
        self.license.write_bytes(b"synthetic MIT notice")
        self.package = self.root / "package"
        self.package.mkdir()

    def source(self, entries):
        path = self.root / "source.tar.gz"
        with tarfile.open(path, "w:gz") as output:
            for name, kind, content in entries:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.size = len(content) if kind == tarfile.REGTYPE else 0
                member.linkname = "../outside"
                output.addfile(member, io.BytesIO(content) if kind == tarfile.REGTYPE else None)
        for key, value in [("SOURCE_BYTES", path.stat().st_size), ("SOURCE_SHA", runtime.digest(path))]:
            patcher = patch.object(runtime, key, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        return path

    def test_options_are_fixed_static_metal_embedded_portable_and_no_dependency_fetch(self):
        for option in ["-DBUILD_SHARED_LIBS=OFF", "-DGGML_NATIVE=OFF", "-DGGML_CPU_ARM_ARCH=armv8-a",
                       "-DGGML_METAL=ON", "-DGGML_METAL_EMBED_LIBRARY=ON", "-DGGML_OPENMP=OFF", "-DGGML_CPU_KLEIDIAI=OFF",
                       "-DCMAKE_OSX_DEPLOYMENT_TARGET=15.0", "-DCMAKE_OSX_ARCHITECTURES=arm64",
                       "-DFETCHCONTENT_FULLY_DISCONNECTED=ON"]:
            self.assertIn(option, runtime.OPTIONS)
        self.assertEqual(runtime.build_info()["sourceSha256"], runtime.SOURCE_SHA)
        self.assertFalse(runtime.build_info()["distributionReady"])

    def test_build_environment_does_not_carry_caller_flags_or_signing_secrets(self):
        with patch.dict(runtime.os.environ, {"CFLAGS": "unwanted", "APPLE_PASSWORD": "private", "DYLD_INSERT_LIBRARIES": "bad"}):
            environment = runtime.environment(Path("/private/tmp/cmake/bin/cmake"))
        self.assertEqual(set(environment), {"PATH", "LC_ALL"})
        self.assertTrue(environment["PATH"].startswith("/private/tmp/cmake/bin:"))

    def test_package_roundtrip_has_exact_notice_provenance_and_non_executable_members(self):
        index = runtime.package(self.binary, self.license, self.package)
        self.assertEqual(runtime.verify_package(self.package), index)
        with tarfile.open(self.package / "whisper-runtime.tar") as source:
            self.assertEqual(source.getnames(), list(runtime.MEMBERS))
            self.assertTrue(all(member.isfile() and member.mode == 0o600 for member in source))
        self.assertEqual(index["runtimeSigning"], "ad_hoc_only")
        self.assertEqual(index["publisherAuthenticity"], "not_verified")

    def test_package_never_overwrites_unknown_destination(self):
        unknown = self.package / "owned-by-user"
        unknown.write_bytes(b"keep")
        with self.assertRaises(ValueError):
            runtime.package(self.binary, self.license, self.package)
        self.assertEqual(unknown.read_bytes(), b"keep")

    def test_signed_metadata_requires_team_but_never_claims_certificate_or_readiness(self):
        index = runtime.package(self.binary, self.license, self.package, signing_team="ABCDE12345")
        self.assertEqual(index["runtimeSigning"], "developer_id_signed")
        self.assertEqual(index["signingTeamId"], "ABCDE12345")
        self.assertEqual(runtime.verify_package(self.package), index)
        self.assertEqual(index["publisherAuthenticity"], "not_verified")
        self.assertFalse(index["distributionReady"])
        path = self.package / "runtime-index.json"
        for changes in [{"signingTeamId": None}, {"signingTeamId": 1234567890},
                        {"signingTeamId": "abcde12345"}, {"signingTeamId": "ABCDE1234"},
                        {"runtimeSigning": "ad_hoc_only"}, {"publisherAuthenticity": "verified"}]:
            path.write_text(json.dumps({**index, **changes}))
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                runtime.verify_package(self.package)
        path.write_text(json.dumps(index))
        with patch.object(runtime.sys, "platform", "darwin"), patch.object(runtime.platform, "machine", return_value="arm64"), patch.object(runtime, "command") as command, self.assertRaises(ValueError):
            runtime.prepare(self.root, None, None, 4)
        command.assert_not_called()

    def test_unpack_creates_only_fixed_checked_bytes_with_no_execution_or_overwrite(self):
        index = runtime.package(self.binary, self.license, self.package)
        destination = self.root / "unpacked"
        destination.mkdir()
        with patch.object(runtime, "command") as command:
            self.assertEqual(runtime.unpack_package(self.package, destination), index)
        command.assert_not_called()
        self.assertEqual((destination / "bin/whisper-cli").read_bytes(), self.binary.read_bytes())
        self.assertEqual((destination / "notices/whisper-MIT.txt").read_bytes(), self.license.read_bytes())
        self.assertEqual(json.loads((destination / "build-info.json").read_bytes()), runtime.build_info())
        self.assertEqual((destination / "bin/whisper-cli").stat().st_mode & 0o777, 0o755)
        with self.assertRaises(ValueError):
            runtime.unpack_package(self.package, destination)

    def test_unpack_checks_tampering_before_creating_any_file(self):
        runtime.package(self.binary, self.license, self.package)
        destination = self.root / "unpacked"
        destination.mkdir()
        archive = self.package / "whisper-runtime.tar"
        archive.write_bytes(archive.read_bytes() + b"changed")
        with self.assertRaises(ValueError):
            runtime.unpack_package(self.package, destination)
        self.assertEqual(list(destination.iterdir()), [])

    def test_unpack_rejects_unsafe_member_or_oversized_notice_even_with_updated_archive_hash(self):
        index = runtime.package(self.binary, self.license, self.package)
        archive = self.package / "whisper-runtime.tar"
        for bad_name, kind, content in [("../outside", tarfile.REGTYPE, b"bad"),
                                       ("bin/whisper-cli", tarfile.SYMTYPE, b""),
                                       ("notices/whisper-MIT.txt", tarfile.REGTYPE, b"x" * 65537)]:
            with tarfile.open(archive, "w") as output:
                for name in runtime.MEMBERS:
                    payload = self.binary.read_bytes() if name == runtime.MEMBERS[0] else self.license.read_bytes() if name == runtime.MEMBERS[1] else json.dumps(runtime.build_info()).encode()
                    replace = name == (runtime.MEMBERS[1] if bad_name == runtime.MEMBERS[1] else runtime.MEMBERS[0])
                    member = tarfile.TarInfo(bad_name if replace else name)
                    member.type = kind if replace else tarfile.REGTYPE
                    member.linkname = "../../outside" if member.issym() else ""
                    payload = content if replace else payload
                    member.size = len(payload)
                    output.addfile(member, io.BytesIO(payload))
            changed = {**index, "archive": {"file": archive.name, "bytes": archive.stat().st_size, "sha256": runtime.digest(archive)}}
            (self.package / "runtime-index.json").write_text(json.dumps(changed))
            with tempfile.TemporaryDirectory(dir=self.root) as temporary, self.subTest(name=bad_name):
                destination = Path(temporary)
                with self.assertRaises(ValueError):
                    runtime.unpack_package(self.package, destination)
                self.assertEqual(list(destination.iterdir()), [])
            self.assertFalse((self.root / "outside").exists())

    def test_changed_archive_is_rejected_without_deletion_or_rebuild(self):
        runtime.package(self.binary, self.license, self.package)
        archive = self.package / "whisper-runtime.tar"
        archive.write_bytes(archive.read_bytes() + b"tampered")
        with self.assertRaises(ValueError):
            runtime.verify_package(self.package)
        self.assertTrue(archive.read_bytes().endswith(b"tampered"))

    def test_changed_version_options_target_or_ready_claim_is_rejected(self):
        runtime.package(self.binary, self.license, self.package)
        path = self.package / "runtime-index.json"
        original = json.loads(path.read_text())
        for key, value in [("sourceCommit", "0" * 40), ("target", "windows_x64"),
                           ("buildOptions", []), ("cpuOnly", True), ("distributionReady", True),
                           ("distributionReady", 0), ("cpuOnly", 1),
                           ("runtimeSigning", "publisher_verified")]:
            path.write_text(json.dumps({**original, key: value}))
            with self.subTest(key=key), self.assertRaises(ValueError):
                runtime.verify_package(self.package)

    def test_unknown_file_and_linked_index_are_rejected(self):
        runtime.package(self.binary, self.license, self.package)
        unknown = self.package / "unknown"
        unknown.write_bytes(b"keep")
        with self.assertRaises(ValueError):
            runtime.verify_package(self.package)
        unknown.unlink()
        path = self.package / "runtime-index.json"
        original = path.read_bytes()
        path.unlink()
        outside = self.root / "external-index"
        outside.write_bytes(original)
        path.symlink_to(outside)
        with self.assertRaises(ValueError):
            runtime.verify_package(self.package)

    def test_source_extract_requires_exact_root_and_creates_only_new_regular_files(self):
        entries = [(runtime.SOURCE_ROOT + "/CMakeLists.txt", tarfile.REGTYPE, b"build"),
                   (runtime.SOURCE_ROOT + "/LICENSE", tarfile.REGTYPE, b"notice")]
        archive = self.source(entries)
        destination = self.root / "source"
        destination.mkdir()
        extracted = runtime.extract_source(archive, destination)
        self.assertEqual((extracted / "LICENSE").read_bytes(), b"notice")
        with self.assertRaises(ValueError):
            runtime.extract_source(archive, destination)

    def test_source_rejects_traversal_links_devices_case_duplicates_and_wrong_root(self):
        for entries in [[("../outside", tarfile.REGTYPE, b"bad")], [("/absolute", tarfile.REGTYPE, b"bad")],
                        [(runtime.SOURCE_ROOT + "/link", tarfile.SYMTYPE, b"")],
                        [(runtime.SOURCE_ROOT + "/hardlink", tarfile.LNKTYPE, b"")],
                        [(runtime.SOURCE_ROOT + "/device", tarfile.CHRTYPE, b"")],
                        [("other/CMakeLists.txt", tarfile.REGTYPE, b"bad")],
                        [(runtime.SOURCE_ROOT + "/x", tarfile.REGTYPE, b"one"), (runtime.SOURCE_ROOT + "/X", tarfile.REGTYPE, b"two")]]:
            archive = self.source(entries)
            with tempfile.TemporaryDirectory(dir=self.root) as destination, self.subTest(entries=entries), self.assertRaises(ValueError):
                runtime.extract_source(archive, Path(destination))

    def test_source_expansion_and_entry_limits_are_enforced(self):
        archive = self.source([(runtime.SOURCE_ROOT + "/x", tarfile.REGTYPE, b"too big")])
        for key, value in [("MAX_EXPANDED", 2), ("MAX_ENTRIES", 0)]:
            with tempfile.TemporaryDirectory(dir=self.root) as destination, patch.object(runtime, key, value), self.assertRaises(ValueError):
                runtime.extract_source(archive, Path(destination))

    def test_wrong_source_hash_and_size_fail_before_compile(self):
        with self.assertRaises(ValueError):
            runtime.check_source(self.binary)
        with patch.object(runtime, "SOURCE_BYTES", self.binary.stat().st_size), self.assertRaises(ValueError):
            runtime.check_source(self.binary)

    def test_checked_cache_does_not_need_cmake_or_network(self):
        runtime.package(self.binary, self.license, self.package)
        with patch.object(runtime.sys, "platform", "darwin"), patch.object(runtime.platform, "machine", return_value="arm64"), patch.object(runtime, "command") as command, patch.object(runtime, "acquire") as acquire:
            result = runtime.prepare(self.root, None, None, 4)
        command.assert_not_called()
        acquire.assert_not_called()
        self.assertEqual(result["state"], "checked_cached_package")

    def test_binary_requires_arm64_15_and_only_system_dependencies(self):
        def tools(argv, **kwargs):
            if "lipo" in argv[0]:
                return b"arm64\n", b""
            if "-L" in argv:
                return b"cli:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1)\n", b""
            if "-l" in argv:
                return b"cmd LC_BUILD_VERSION\n minos 15.0\n", b""
            return b"", b""
        with patch.object(runtime, "command", side_effect=tools):
            runtime.verify_binary(self.binary)
        for bad in [b"x86_64\n", b"arm64 x86_64\n"]:
            with patch.object(runtime, "command", return_value=(bad, b"")), self.assertRaises(ValueError):
                runtime.verify_binary(self.binary)
        def external(argv, **kwargs):
            return (b"cli:\n\t@rpath/libuser.dylib (compatibility version 1)\n", b"") if "-L" in argv else tools(argv, **kwargs)
        with patch.object(runtime, "command", side_effect=external), self.assertRaises(ValueError):
            runtime.verify_binary(self.binary)

    def test_build_configuration_includes_only_sealed_package_and_no_model_weights(self):
        config = json.loads((runtime.REPOSITORY / "app/src-tauri/tauri.macos.conf.json").read_text())
        resources = config["bundle"]["resources"]
        self.assertEqual(resources["../../.cache/whisper-metal-runtime/package/whisper-runtime.tar"], "asr-runtime/whisper-runtime.tar")
        self.assertEqual(resources["../../.cache/whisper-metal-runtime/package/runtime-index.json"], "asr-runtime/runtime-index.json")
        self.assertFalse(any("ggml-" in path or "compile-" in path for path in resources))
        self.assertIn("mon_whisper_runtime.py", (runtime.REPOSITORY / "app/build-macos.sh").read_text())

    def test_source_transport_parent_enforces_capacity_without_content_length(self):
        pending = self.root / "streamed-source"
        runtime.download_source([sys.executable, "-c", "import sys; sys.stdout.buffer.write(b'fixed')"], pending, 5)
        self.assertEqual(pending.read_bytes(), b"fixed")
        oversized = self.root / "oversized-source"
        with self.assertRaises(ValueError):
            runtime.download_source([sys.executable, "-c", "import sys; sys.stdout.buffer.write(b'x'*100000)"], oversized, 5)
        self.assertLessEqual(oversized.stat().st_size, 5)

    def test_source_transport_timeout_reaps_only_its_owned_child(self):
        # An owned child blocked on input stays alive until the parent's exact
        # virtual deadline; neither interpreter startup nor a sleep is a clock.
        process = runtime.subprocess.Popen
        children = []
        def spawn(argv, **kwargs):
            kwargs["stdin"] = runtime.subprocess.PIPE
            child = process(argv, **kwargs)
            children.append(child)
            return child
        with (patch.object(runtime.subprocess, "Popen", side_effect=spawn),
              patch.object(runtime.time, "monotonic", side_effect=[0.0, 130.0]),
              self.assertRaisesRegex(ValueError, "exceeded its time limit")):
            runtime.download_source([sys.executable, "-c", "import sys; sys.stdin.buffer.read()"],
                                    self.root / "timeout-source", 5)
        self.assertEqual(len(children), 1)
        self.assertIsNotNone(children[0].poll(), "the timed-out owned child must be reaped")
        children[0].stdin.close()


if __name__ == "__main__":
    unittest.main()
