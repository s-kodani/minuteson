import importlib.util
import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from contextlib import ExitStack, redirect_stderr, redirect_stdout
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "mon_macos_release", Path(__file__).resolve().parents[1] / "tools/mon_macos_release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
TEAM = "ABCDEFGHIJ"
IDENTITY = f"Developer ID Application: Test Team ({TEAM})"
REQUEST = "a3a0c196-188e-4ec0-99d2-55be4cb4e327"


class ReleaseConfigurationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.config = self.root / "app/src-tauri"
        self.config.mkdir(parents=True)
        self.write("tauri.conf.json", {"productName": "MinutesOn", "version": "1.0.0",
                                      "identifier": "jp.mon.desktop"})
        self.write("tauri.macos.conf.json", {})
        self.write("tauri.release.conf.json", {})

    def write(self, name, value):
        (self.config / name).write_text(json.dumps(value), encoding="utf8")

    def test_release_name_and_version_follow_tauri_platform_and_explicit_overrides(self):
        self.write("tauri.macos.conf.json", {"productName": "MinutesOn Mac",
                                           "bundle": {"targets": ["app"]}})
        self.write("tauri.release.conf.json", {"productName": "MinutesOn", "version": "1.0.1",
                                             "bundle": {"active": True}})
        config = release.release_configuration(self.root)
        self.assertEqual(config["productName"], "MinutesOn")
        self.assertEqual(config["version"], "1.0.1")
        self.assertEqual(config["identifier"], "jp.mon.desktop")
        self.assertEqual(config["bundle"], {"targets": ["app"], "active": True})

    def test_unsafe_product_names_cannot_escape_or_change_artifact_paths(self):
        for name in [None, "", " ", "MinutesOn ", ".", "..", "../MinutesOn", "a/b", "a\\b",
                     "a:b", "-option", "MinutesOn.", "MinutesOn\n", "a" * 129]:
            with self.subTest(name=name):
                self.write("tauri.release.conf.json", {"productName": name})
                with self.assertRaises(ValueError):
                    release.release_configuration(self.root)

    def test_release_configuration_requires_numeric_version_and_object_overrides(self):
        for value in [{"version": "1.0.0-beta"}, {"version": 1}, {"version": None}, []]:
            with self.subTest(value=value):
                self.write("tauri.release.conf.json", value)
                with self.assertRaises(ValueError):
                    release.release_configuration(self.root)

    def test_shipped_privacy_explanations_use_formal_product_name(self):
        for name in ("app/src-tauri/release-entitlements/Info.plist", "probes/macos-capture/Info.plist"):
            info = plistlib.loads((release.REPOSITORY / name).read_bytes())
            for key in ("NSMicrophoneUsageDescription", "NSAudioCaptureUsageDescription"):
                with self.subTest(plist=name, key=key):
                    self.assertIn("MinutesOn", info[key])
        self.assertEqual(info["CFBundleName"], "MinutesOn Capture Helper")
        self.assertEqual(info["CFBundleDisplayName"], "MinutesOn Capture Helper")
        self.assertEqual(info["CFBundleIdentifier"], "app.mon.capture.probe")
        self.assertEqual(info["CFBundleExecutable"], "mon-macos-capture")

    def test_pipeline_uses_effective_name_for_bundle_signing_and_disk_image(self):
        # Only the macOS command boundary is synthetic; actual copies, paths,
        # config merging, result recording and cache cleanup run on this host.
        self.write("tauri.release.conf.json", {"productName": "MinutesOn Preview"})
        policy = self.config / "release-entitlements"
        policy.mkdir()
        for name, value in release.POLICIES.items():
            (policy / name).write_bytes(plistlib.dumps(value))
        commands = []
        signed = []

        def command(argv, **kwargs):
            commands.append(argv)
            if Path(argv[0]).name == "tauri":
                built = Path(kwargs["env"]["CARGO_TARGET_DIR"]) / "release/bundle/macos/MinutesOn Preview.app"
                built.mkdir(parents=True)
                (built / "source").write_bytes(b"synthetic application")
            elif Path(argv[0]).name == "ditto":
                shutil.copytree(argv[-2], argv[-1])
            elif "--destination" in argv:
                Path(argv[argv.index("--destination") + 1]).write_bytes(b"synthetic disk image")
            return b"", b""

        with ExitStack() as stack:
            stack.enter_context(patch.object(release, "REPOSITORY", self.root))
            stack.enter_context(patch.object(release.sys, "platform", "darwin"))
            stack.enter_context(patch.object(release, "command", side_effect=command))
            stack.enter_context(patch.object(release.shutil, "disk_usage", return_value=type("Disk", (), {"free": 8 * 1024 ** 3})()))
            stack.enter_context(patch.object(release, "layout", side_effect=lambda app, version, name: (
                app / "Contents/Resources/MONCaptureProbe.app", app / "Contents/Resources/mon-speaker-embedding")))
            for name in ("verify_notices", "verify_asr_runtime", "verify_inference_runtime"):
                stack.enter_context(patch.object(release, name, return_value={}))
            for name in ("sign_asr_runtime", "sign_inference_runtime", "verify_signature"):
                stack.enter_context(patch.object(release, name))
            stack.enter_context(patch.object(release, "sign", side_effect=lambda path, *args: signed.append(path.name)))
            # Creating the Applications alias itself is covered by POSIX tests.
            stack.enter_context(patch.object(Path, "symlink_to"))
            output = self.root / "distribution"
            result = release.build(output, IDENTITY, TEAM, None, {})

        self.assertTrue((output / "MinutesOn Preview.app/source").is_file())
        self.assertTrue((output / "image-source/MinutesOn Preview.app/source").is_file())
        self.assertEqual(signed, ["MONCaptureProbe.app", "mon-speaker-embedding", "MinutesOn Preview.app",
                                  "MinutesOn Preview_1.0.0_aarch64.dmg"])
        creation = next(argv for argv in commands if "--destination" in argv)
        self.assertEqual(creation[creation.index("--product-name") + 1], "MinutesOn Preview")
        self.assertEqual(result["productName"], "MinutesOn Preview")
        self.assertEqual(result["dmg"], "MinutesOn Preview_1.0.0_aarch64.dmg")
        self.assertFalse(result["distributionReady"])
        self.assertFalse(Path(result["buildCache"]).exists())


@unittest.skipIf(os.name == "nt", "The macOS release fixtures require POSIX runtime packaging and permissions")
class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.app = self.root / "MinutesOn.app"
        self.app.mkdir()
        self.calls = []
        main = {"CFBundleIdentifier": "jp.mon.desktop", "CFBundleShortVersionString": "1.0.0", "CFBundleVersion": "1.0.0",
                "CFBundleName": "MinutesOn", "CFBundleDisplayName": "MinutesOn",
                "LSMinimumSystemVersion": "15.0", "CFBundleExecutable": "mon-desktop",
                "NSMicrophoneUsageDescription": "microphone", "NSAudioCaptureUsageDescription": "audio"}
        self.plist(self.app / "Contents/Info.plist", main)
        self.file(self.app / "Contents/MacOS/mon-desktop")
        self.capture = self.app / "Contents/Resources/MONCaptureProbe.app"
        self.plist(self.capture / "Contents/Info.plist", {
            "CFBundleIdentifier": "app.mon.capture.probe", "CFBundleExecutable": "mon-macos-capture",
            "CFBundleName": "MinutesOn Capture Helper", "CFBundleDisplayName": "MinutesOn Capture Helper",
            "NSMicrophoneUsageDescription": "microphone", "NSAudioCaptureUsageDescription": "audio"})
        self.file(self.capture / "Contents/MacOS/mon-macos-capture")
        self.file(self.capture / "Contents/Resources/PrivacyInfo.xcprivacy")
        self.embedding = self.file(self.app / "Contents/Resources/mon-speaker-embedding")
        self.notice_text = self.file(self.app / "Contents/Resources/THIRD_PARTY_NOTICES.txt", b"synthetic notices")
        self.notice_inventory = self.app / "Contents/Resources/third-party-inventory.json"
        self.inventory = {"schemaVersion": 1, "target": "aarch64-apple-darwin",
                          "licenseReview": "not_verified", "distributionReady": False,
                          "supplementManifestSha256": release.digest(release.REPOSITORY / "third-party/notices/index.json"),
                          "lockHashes": {name: release.digest(release.REPOSITORY / relative) for name, relative in
                                         (("cargo", "Cargo.lock"), ("npm", "app/package-lock.json"))},
                          "packages": [{"name": "synthetic"}], "missingNoticeTexts": ["synthetic"],
                          "noticesSha256": release.digest(self.notice_text), "noticesBytes": self.notice_text.stat().st_size}
        self.file(self.notice_inventory, json.dumps(self.inventory).encode())
        expected = patch.object(release, "expected_notices", return_value=(
            self.notice_text.read_bytes(), json.loads(json.dumps(self.inventory))))
        expected.start()
        self.addCleanup(expected.stop)
        runtime_spec = importlib.util.spec_from_file_location("release_test_runtime", release.REPOSITORY / "tools/mon_whisper_runtime.py")
        runtime = importlib.util.module_from_spec(runtime_spec)
        runtime_spec.loader.exec_module(runtime)
        asr_directory = self.app / "Contents/Resources/asr-runtime"
        asr_directory.mkdir()
        runtime.package(self.embedding, self.notice_text, asr_directory)
        for executable in (self.app / "Contents/MacOS/mon-desktop", self.capture / "Contents/MacOS/mon-macos-capture", self.embedding):
            executable.chmod(0o755)
        workers = self.app / "Contents/Resources/inference-runtime"
        entries = []
        for engine, name in zip(("whisper", "llama", "sherpa", "asr-transducer"), release.WORKER_NAMES):
            binary = self.file(workers / name, ("synthetic " + name).encode())
            binary.chmod(0o755)
            entries.append({"protocol":1,"engine":engine,"name":name,"bytes":binary.stat().st_size,"sha256":release.digest(binary)})
        self.file(workers / "runtime-index.json", json.dumps(entries).encode())
        for name in ("whisper-MIT.txt", "llama-MIT.txt", "nlohmann-MIT.txt", "sherpa-Apache-2.0.txt", "build-info.json"):
            self.file(workers / name)
        self.policy = release.REPOSITORY / "app/src-tauri/release-entitlements"

    def file(self, path, value=b"synthetic"):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(value)
        return path

    def plist(self, path, value):
        return self.file(path, plistlib.dumps(value))

    def signed(self, argv, **kwargs):
        self.calls.append(argv)
        if "-archs" in argv:
            return b"arm64\n", b""
        if Path(argv[0]).name == "otool":
            if "-L" in argv:
                return b"cli:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1)\n", b""
            return b"cmd LC_BUILD_VERSION\n minos 15.0\n", b""
        if "--sign" in argv and Path(argv[-1]).name == "whisper-cli":
            # Synthetic signature stand-in changes bytes. No real signing key,
            # executable, ticket or distribution proof is used in this test.
            binary = Path(argv[-1])
            binary.write_bytes(binary.read_bytes() + b" synthetic Developer ID signature")
        if "--sign" in argv and Path(argv[-1]).name == "MinutesOn.app":
            # Repacking must finish before the enclosing resource seal is made.
            directory = Path(argv[-1]) / "Contents/Resources/asr-runtime"
            index = release.asr_module().verify_package(directory)
            self.assertEqual(index["runtimeSigning"], "developer_id_signed")
            self.assertEqual(index["signingTeamId"], TEAM)
        if "--entitlements" in argv and "--display" in argv:
            target = argv[-1]
            policy = "embedding.plist" if target.endswith(("mon-speaker-embedding", "mon-sherpa-worker", "mon-transducer-worker")) else "capture.plist" if target.endswith("MONCaptureProbe.app") else "main.plist"
            return (self.policy / policy).read_bytes(), b"warning: display"
        if "--display" in argv:
            return b"", (f"Authority={IDENTITY}\nTeamIdentifier={TEAM}\nTimestamp=Oct 10 2026\nCodeDirectory v=20500 size=100 flags=0x10000(runtime) hashes=3+2\n").encode()
        return b"", b""

    def test_identity_requires_explicit_matching_developer_id_and_team(self):
        release.credentials(IDENTITY, TEAM, None)
        for identity, team in [("-", TEAM), ("Apple Development: Test", TEAM),
                               (IDENTITY, "KLMNOPQRST"), (IDENTITY, "bad"),
                               (IDENTITY + "\n", TEAM), ("", TEAM)]:
            with self.subTest(identity=identity, team=team), self.assertRaises(ValueError):
                release.credentials(identity, team, None)

    def test_profile_rejects_empty_control_and_overlong_names(self):
        for profile in ["", " ", "x\n", "-option", "x" * 129]:
            with self.subTest(profile=profile), self.assertRaises(ValueError):
                release.credentials(IDENTITY, TEAM, profile)
        release.credentials(IDENTITY, TEAM, "explicit-notary-profile")

    def test_tauri_environment_removes_all_signing_and_notary_credentials_without_mutating_parent(self):
        source = {key: "private" for key in (*release.NOTARY_ENV, "APPLE_SIGNING_IDENTITY")}
        source.update(PATH="keep", CARGO_TARGET_DIR="old", CARGO_INCREMENTAL="1")
        result = release.build_environment(source, self.root)
        self.assertEqual(result["PATH"], "keep")
        self.assertEqual(result["CARGO_TARGET_DIR"], str(self.root))
        self.assertEqual(result["CARGO_INCREMENTAL"], "0")
        self.assertFalse(set(result) & set((*release.NOTARY_ENV, "APPLE_SIGNING_IDENTITY")))
        self.assertEqual(source["APPLE_PASSWORD"], "private")

    def test_missing_credentials_fail_before_any_file_build_sign_or_upload(self):
        target = self.root / "not-created"
        with patch.object(release, "command") as command, self.assertRaises(ValueError):
            release.build(target, "", TEAM, None, {})
        command.assert_not_called()
        self.assertFalse(target.exists())

    def test_existing_destination_is_never_overwritten(self):
        with patch.object(release.sys, "platform", "darwin"), patch.object(release, "command") as command, self.assertRaises(ValueError):
            release.build(self.app, IDENTITY, TEAM, None, {})
        command.assert_not_called()
        self.assertEqual((self.app / "Contents/MacOS/mon-desktop").read_bytes(), b"synthetic")

    def test_layout_checks_actual_binary_name_identifier_version_privacy_helpers_and_architecture(self):
        with patch.object(release, "command", side_effect=self.signed):
            self.assertEqual(release.layout(self.app, "1.0.0", "MinutesOn"), (self.capture, self.embedding))
        self.assertEqual(sum("-archs" in argv for argv in self.calls), 3)
        info_path = self.app / "Contents/Info.plist"
        original = release.read_plist(info_path)
        for key, value in [("CFBundleIdentifier", "jp.mon.desktop.qa"), ("CFBundleExecutable", "mon"),
                           ("CFBundleName", "mon"), ("CFBundleDisplayName", "MinutesOn QA"),
                           ("CFBundleShortVersionString", "0.2.0"), ("LSMinimumSystemVersion", "14.0"),
                           ("CFBundleVersion", "0.2.0"), ("NSMicrophoneUsageDescription", ""),
                           ("NSMicrophoneUsageDescription", True), ("NSAudioCaptureUsageDescription", "")]:
            self.plist(info_path, {**original, key: value})
            with self.subTest(key=key), patch.object(release, "command") as command, self.assertRaises(ValueError):
                release.layout(self.app, "1.0.0", "MinutesOn")
            command.assert_not_called()
        self.plist(info_path, original)
        with patch.object(release, "command", return_value=(b"x86_64 arm64", b"")), self.assertRaises(ValueError):
            release.layout(self.app, "1.0.0", "MinutesOn")
        self.embedding.chmod(0o644)
        with patch.object(release, "command", side_effect=self.signed), self.assertRaises(ValueError):
            release.layout(self.app, "1.0.0", "MinutesOn")

    def test_layout_rejects_missing_and_linked_helper(self):
        for linked in [False, True]:
            self.embedding.unlink(missing_ok=True)
            if linked:
                self.embedding.symlink_to(self.app / "Contents/MacOS/mon-desktop")
            with self.subTest(linked=linked), patch.object(release, "command") as command, self.assertRaises(ValueError):
                release.layout(self.app, "1.0.0", "MinutesOn")
            command.assert_not_called()

    def test_signs_one_component_not_deep_and_verifies_apple_anchor_team_runtime_timestamp_and_policy(self):
        with patch.object(release, "command", side_effect=self.signed):
            release.sign(self.capture, IDENTITY, TEAM, self.policy / "capture.plist")
        sign = self.calls[0]
        self.assertNotIn("--deep", sign)
        self.assertIn("--timestamp", sign)
        self.assertIn("runtime", sign)
        verify = self.calls[1]
        self.assertIn("--deep", verify)
        self.assertIn("--strict", verify)
        self.assertIn(release.requirement(TEAM), verify)
        self.assertIn("anchor apple generic", release.requirement(TEAM))
        self.assertTrue(release.requirement(TEAM).startswith("=anchor apple generic"))
        self.assertIn("100.6.1.13", release.requirement(TEAM))

    def test_signature_text_cannot_replace_native_requirement_verification(self):
        with patch.object(release, "command", side_effect=ValueError("native refusal")), self.assertRaises(ValueError):
            release.verify_signature(self.app, TEAM, runtime=True)

    def test_signature_rejects_wrong_team_ad_hoc_missing_timestamp_and_runtime(self):
        base = f"Authority={IDENTITY}\nTeamIdentifier={TEAM}\nTimestamp=now\nflags=0x10000(runtime)\n"
        for details in [base.replace(TEAM, "KLMNOPQRST"), base.replace(f"Authority={IDENTITY}", "Signature=adhoc"),
                        base.replace("Timestamp=now\n", ""), base.replace("0x10000", "0x0")]:
            with self.subTest(details=details), patch.object(release, "command", side_effect=[(b"", b""), (b"", details.encode())]), self.assertRaises(ValueError):
                release.verify_signature(self.app, TEAM, runtime=True)

    def test_entitlement_mismatch_and_extra_debug_or_library_exception_are_rejected(self):
        for value in [{}, {"com.apple.security.get-task-allow": True},
                      {"com.apple.security.device.audio-input": True, "com.apple.security.cs.disable-library-validation": True}]:
            def command(argv, **kwargs):
                if "--entitlements" in argv:
                    return plistlib.dumps(value), b""
                return self.signed(argv, **kwargs)
            with self.subTest(value=value), patch.object(release, "command", side_effect=command), self.assertRaises(ValueError):
                release.verify_signature(self.capture, TEAM, runtime=True, entitlements=self.policy / "capture.plist")

    def test_notary_requires_accepted_response_and_explicit_profile_not_environment_secrets(self):
        with patch.object(release, "command", return_value=(json.dumps({"status": "Accepted", "id": REQUEST}).encode(), b"")) as command:
            self.assertEqual(release.notarize(self.embedding, "my-profile"), REQUEST)
        self.assertIn("--keychain-profile", command.call_args.args[0])
        self.assertIn("my-profile", command.call_args.args[0])
        for response in [b'{"status":"Invalid","id":"request"}', b'{"status":"In Progress","id":"request"}',
                         b'{"status":"Accepted"}', b'{"status":"Accepted","id":"not-a-request"}', b'[]', b'{}', b'not-json']:
            with self.subTest(response=response), patch.object(release, "command", return_value=(response, b"")), self.assertRaises(ValueError):
                release.notarize(self.embedding, "my-profile")

    def test_cli_requires_one_explicit_mode_and_missing_credentials_do_not_print_secrets(self):
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            release.main(["--output-dir", str(self.root / "out"), "--team-id", TEAM])
        error = io.StringIO()
        with patch.dict(os.environ, {"APPLE_SIGNING_IDENTITY": "", "APPLE_PASSWORD": "do-not-print"}), redirect_stderr(error), redirect_stdout(io.StringIO()):
            self.assertEqual(release.main(["--output-dir", str(self.root / "out"), "--team-id", TEAM, "--sign-only"]), 1)
        self.assertNotIn("do-not-print", error.getvalue())
        self.assertFalse((self.root / "out").exists())

    def test_owned_command_failure_timeout_and_interrupt_do_not_claim_success(self):
        with self.assertRaises(ValueError):
            release.command(["/bin/sh", "-c", "exit 2"])
        with self.assertRaises(subprocess.TimeoutExpired):
            release.command(["/bin/sleep", "10"], timeout=0.05)
        with patch.object(release.subprocess, "Popen") as popen, patch.object(release.os, "killpg") as kill:
            process = popen.return_value
            process.pid = 12345
            process.communicate.side_effect = [KeyboardInterrupt(), (b"", b"")]
            with self.assertRaises(KeyboardInterrupt):
                release.command(["synthetic-owned-child"])
            kill.assert_called_once_with(12345, release.signal.SIGTERM)

    def pipeline(self, profile=None, fail_at=None):
        output = self.root / "distribution"
        def command(argv, **kwargs):
            self.calls.append(argv)
            if fail_at is not None and fail_at in argv:
                raise ValueError("synthetic phase failure")
            if Path(argv[0]).name == "tauri":
                cache = Path(kwargs["env"]["CARGO_TARGET_DIR"])
                shutil.copytree(self.app, cache / "release/bundle/macos/MinutesOn.app")
                self.assertFalse(set(kwargs["env"]) & set((*release.NOTARY_ENV, "APPLE_SIGNING_IDENTITY")))
            elif Path(argv[0]).name == "ditto":
                if "-c" in argv:
                    Path(argv[-1]).write_bytes(b"synthetic notary archive")
                else:
                    shutil.copytree(argv[-2], argv[-1])
            elif "--destination" in argv:
                Path(argv[argv.index("--destination") + 1]).write_bytes(b"synthetic disk image")
            elif "notarytool" in argv:
                return json.dumps({"status": "Accepted", "id": REQUEST}).encode(), b""
            return self.signed(argv, **kwargs)
        # signed() records too, retain one command record per invocation below.
        def single(argv, **kwargs):
            before = len(self.calls)
            result = command(argv, **kwargs)
            if len(self.calls) == before + 2:
                self.calls.pop()
            return result
        with patch.object(release.sys, "platform", "darwin"), patch.object(release, "release_configuration", return_value={"productName": "MinutesOn", "version": "1.0.0"}), patch.object(release, "command", side_effect=single), patch.object(release.shutil, "disk_usage", return_value=type("Disk", (), {"free": 8 * 1024 ** 3})()):
            return output, release.build(output, IDENTITY, TEAM, profile, {"APPLE_PASSWORD": "secret", "PATH": "keep"})

    def test_sign_only_pipeline_is_inside_out_creates_new_dmg_and_never_uploads_staples_or_launches(self):
        output, result = self.pipeline()
        signs = [argv for argv in self.calls if "--sign" in argv]
        self.assertEqual([Path(argv[-1]).name for argv in signs], ["whisper-cli", *release.WORKER_NAMES, "MONCaptureProbe.app", "mon-speaker-embedding", "MinutesOn.app", "MinutesOn_1.0.0_aarch64.dmg"])
        creation = next(argv for argv in self.calls if "--destination" in argv)
        self.assertEqual(creation[creation.index("--product-name") + 1], "MinutesOn")
        self.assertEqual(result["productName"], "MinutesOn")
        self.assertFalse(any("notarytool" in argv or "stapler" in argv or Path(argv[0]).name == "open" for argv in self.calls))
        self.assertEqual(result["notarization"], "not_requested")
        self.assertFalse(result["distributionReady"])
        self.assertEqual(result["runtimeQA"], "not_verified")
        self.assertEqual([result[key] for key in ["gatekeeper", "licenseReview", "manualUpdate"]], ["not_verified"] * 3)
        self.assertEqual(result["dependencyNotices"]["state"], "collected_not_reviewed")
        self.assertEqual(result["dependencyNotices"]["missingNoticeTexts"], ["synthetic"])
        self.assertEqual(result["asrRuntime"]["state"], "packaged_onboarding_unverified")
        self.assertEqual(result["asrRuntime"]["runtimeSigning"], "developer_id_signed")
        self.assertEqual(result["asrRuntime"]["developerIdSigning"], "verified")
        self.assertEqual(result["asrRuntime"]["standaloneNotarization"], "not_verified")
        self.assertEqual(result["dmgSha256"], release.digest(output / result["dmg"]))
        self.assertEqual(json.loads((output / "build-result.json").read_text()), result)
        self.assertEqual((output / "image-source/Applications").readlink(), Path("/Applications"))
        self.assertNotIn("secret", (output / "build-result.json").read_text())
        self.assertFalse(Path(result["buildCache"]).exists())
        self.assertFalse(result["buildCacheRetained"])
        self.assertEqual([p.name for p in output.iterdir() if p.name.startswith("mon-asr-")], [])
        runtime = release.asr_module()
        original, old_files = runtime.package_contents(self.app / "Contents/Resources/asr-runtime")
        sealed, files = runtime.package_contents(output / "MinutesOn.app/Contents/Resources/asr-runtime")
        self.assertEqual(original["runtimeSigning"], "ad_hoc_only")
        self.assertEqual(sealed["signingTeamId"], TEAM)
        self.assertNotEqual(files["bin/whisper-cli"], old_files["bin/whisper-cli"])
        self.assertEqual(sealed["files"][1:], original["files"][1:])
        self.assertNotEqual(sealed["archive"]["sha256"], original["archive"]["sha256"])

    def test_explicit_notary_pipeline_staples_app_before_dmg_and_never_rebuilds_accepted_dmg(self):
        _, result = self.pipeline("existing-explicit-profile")
        uploads = [i for i, argv in enumerate(self.calls) if "notarytool" in argv]
        creation = next(i for i, argv in enumerate(self.calls) if "--destination" in argv)
        app_staple = next(i for i, argv in enumerate(self.calls) if "stapler" in argv and "staple" in argv and argv[-1].endswith("MinutesOn.app"))
        dmg_staple = next(i for i, argv in enumerate(self.calls) if "stapler" in argv and "staple" in argv and argv[-1].endswith(".dmg"))
        self.assertEqual(len(uploads), 2)
        self.assertLess(uploads[0], app_staple)
        self.assertLess(app_staple, creation)
        self.assertLess(creation, uploads[1])
        self.assertLess(uploads[1], dmg_staple)
        self.assertEqual(sum("--destination" in argv for argv in self.calls), 1)
        self.assertEqual(result["notaryRequestIds"], [REQUEST, REQUEST])
        self.assertEqual(result["notarization"], "tickets_validated")
        self.assertFalse(result["distributionReady"])

    def test_pipeline_failure_preserves_partial_artifacts_without_publishing_success_record(self):
        with redirect_stderr(io.StringIO()), self.assertRaises(ValueError):
            self.pipeline(fail_at="--destination")
        output = self.root / "distribution"
        self.assertTrue((output / "MinutesOn.app").is_dir())
        self.assertTrue((output / "preparation-build.log").is_file())
        self.assertFalse((output / "build-result.json").exists())

    def test_missing_dmg_dependencies_fail_before_building_or_signing(self):
        with self.assertRaises(ValueError):
            self.pipeline(fail_at="--check")
        self.assertFalse((self.root / "distribution").exists())
        self.assertFalse(any("--sign" in argv or "notarytool" in argv for argv in self.calls))

    def test_changed_policy_refused_before_build_and_target_creation(self):
        original = release.read_plist
        def read(path):
            return {"com.apple.security.get-task-allow": True} if path.name == "main.plist" else original(path)
        target = self.root / "out"
        with patch.object(release.sys, "platform", "darwin"), patch.object(release, "read_plist", side_effect=read), patch.object(release, "command") as command, patch.object(release.shutil, "disk_usage", return_value=type("Disk", (), {"free": 8 * 1024 ** 3})()), self.assertRaises(ValueError):
            release.build(target, IDENTITY, TEAM, None, {})
        command.assert_not_called()
        self.assertFalse(target.exists())

    def test_parent_and_capture_signing_allow_only_audio_input(self):
        expected = {"com.apple.security.device.audio-input": True}
        for name in ("main.plist", "capture.plist"):
            with self.subTest(name=name):
                self.assertEqual(release.read_plist(self.policy / name), expected)
                self.assertEqual(release.POLICIES[name], expected)

    def test_missing_parent_audio_input_is_rejected_before_build(self):
        original = release.read_plist
        def read(path):
            return {} if path.name == "main.plist" else original(path)
        target = self.root / "missing-audio-policy"
        with patch.object(release.sys, "platform", "darwin"), patch.object(release, "read_plist", side_effect=read), patch.object(release, "command") as command, patch.object(release.shutil, "disk_usage", return_value=type("Disk", (), {"free": 8 * 1024 ** 3})()), self.assertRaises(ValueError):
            release.build(target, IDENTITY, TEAM, None, {})
        command.assert_not_called()
        self.assertFalse(target.exists())

    def test_notices_missing_tampered_stale_or_self_approved_are_rejected(self):
        self.assertEqual(release.verify_notices(self.app)["state"], "collected_not_reviewed")
        for key, value in [("lockHashes", {}), ("noticesSha256", "0" * 64), ("noticesBytes", 0),
                           ("supplementManifestSha256", "0" * 64),
                           ("licenseReview", "approved"), ("distributionReady", True),
                           ("packages", []), ("missingNoticeTexts", None)]:
            self.file(self.notice_inventory, json.dumps({**self.inventory, key: value}).encode())
            with self.subTest(key=key), self.assertRaises(ValueError):
                release.verify_notices(self.app)
        self.file(self.notice_inventory, json.dumps(self.inventory).encode())
        self.notice_text.write_bytes(b"changed")
        with self.assertRaises(ValueError):
            release.verify_notices(self.app)
        self.notice_text.unlink()
        with self.assertRaises(ValueError):
            release.verify_notices(self.app)

    def test_notice_collection_failure_refuses_before_any_sign_or_upload(self):
        self.notice_text.unlink()
        with redirect_stderr(io.StringIO()), self.assertRaises(ValueError):
            self.pipeline("existing-explicit-profile")
        self.assertFalse(any("--sign" in argv or "notarytool" in argv for argv in self.calls))

    def test_notice_metadata_cannot_hide_missing_texts_or_replace_package_provenance(self):
        for key, value in [("missingNoticeTexts", []),
                           ("packages", [{"name": "foreign-but-nonempty"}])]:
            self.file(self.notice_inventory, json.dumps({**self.inventory, key: value}).encode())
            with self.subTest(key=key), self.assertRaises(ValueError):
                release.verify_notices(self.app)

    def test_notice_recollection_is_offline_locked_read_only_and_uses_actual_inputs(self):
        # The pipeline tests use a synthetic recollection boundary. Exercise
        # that boundary's real implementation separately without Cargo/network.
        spec = importlib.util.spec_from_file_location("release_recollection_test", SPEC.origin)
        actual = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(actual)
        metadata = b'{"resolve":{"root":null,"nodes":[]},"packages":[]}'
        with patch.object(actual, "command", return_value=(metadata, b"")) as command, self.assertRaises(ValueError):
            actual.expected_notices()
        command.assert_called_once_with(["cargo", "metadata", "--offline", "--locked", "--format-version", "1",
                                         "--filter-platform", "aarch64-apple-darwin"],
                                        cwd=actual.REPOSITORY / "app/src-tauri", timeout=120)

    def test_notice_metadata_mismatch_refuses_pipeline_before_sign_or_upload(self):
        self.file(self.notice_inventory, json.dumps({**self.inventory, "missingNoticeTexts": []}).encode())
        with redirect_stderr(io.StringIO()), self.assertRaises(ValueError):
            self.pipeline("existing-explicit-profile")
        self.assertFalse(any("--sign" in argv or "notarytool" in argv for argv in self.calls))
        self.assertFalse((self.root / "distribution/build-result.json").exists())

    def test_asr_resource_verification_does_not_claim_inner_cli_signed_or_notarized(self):
        proof = release.verify_asr_runtime(self.app)
        self.assertEqual(proof["runtimeSigning"], "ad_hoc_only")
        self.assertEqual(proof["developerIdSigning"], "not_verified")
        self.assertEqual(proof["nativeOnboarding"], "not_verified")

    def test_changed_worker_refuses_release_before_any_signing(self):
        binary = self.app / "Contents/Resources/inference-runtime/mon-whisper-worker"
        binary.write_bytes(binary.read_bytes() + b"changed")
        with self.assertRaises(ValueError):
            self.pipeline()
        self.assertFalse(any("--sign" in argv for argv in self.calls))

    def test_worker_index_cannot_reference_a_path_outside_resources(self):
        index = self.app / "Contents/Resources/inference-runtime/runtime-index.json"
        entries = json.loads(index.read_text())
        entries[0]["name"] = "../mon-speaker-embedding"
        index.write_text(json.dumps(entries))
        with self.assertRaises(ValueError):
            release.verify_inference_runtime(self.app)

    def test_changed_asr_archive_refuses_release_before_sign_or_upload(self):
        archive = self.app / "Contents/Resources/asr-runtime/whisper-runtime.tar"
        archive.write_bytes(archive.read_bytes() + b"changed")
        with self.assertRaises(ValueError):
            self.pipeline()
        self.assertFalse(any("--sign" in argv or "notarytool" in argv for argv in self.calls))
        self.assertFalse((self.root / "distribution/build-result.json").exists())

    def test_asr_signing_failure_never_signs_outer_app_or_uploads_or_changes_source_package(self):
        source = self.app / "Contents/Resources/asr-runtime/whisper-runtime.tar"
        before = source.read_bytes()
        original = self.signed
        def refuse(argv, **kwargs):
            if "--sign" in argv and Path(argv[-1]).name == "whisper-cli":
                raise ValueError("synthetic missing signing key")
            return original(argv, **kwargs)
        with patch.object(self, "signed", side_effect=refuse), redirect_stderr(io.StringIO()), self.assertRaises(ValueError):
            self.pipeline("existing-explicit-profile")
        self.assertEqual(source.read_bytes(), before)
        self.assertFalse(any("notarytool" in argv or ("--sign" in argv and argv[-1].endswith("MinutesOn.app")) for argv in self.calls))
        self.assertFalse((self.root / "distribution/build-result.json").exists())

    def test_asr_metadata_alone_cannot_substitute_for_native_certificate_verification(self):
        runtime = release.asr_module()
        directory = self.app / "Contents/Resources/asr-runtime"
        index_path = directory / "runtime-index.json"
        index = json.loads(index_path.read_text())
        index.update(runtimeSigning="developer_id_signed", signingTeamId=TEAM)
        index_path.write_text(json.dumps(index))
        # Without an expected Team and actual OS checks, the label proves nothing.
        self.assertEqual(release.verify_asr_runtime(self.app)["developerIdSigning"], "not_verified")
        def reject_certificate(argv, **kwargs):
            if "--verify" in argv and "-R" in argv:
                raise ValueError("native certificate requirement refusal")
            return self.signed(argv, **kwargs)
        with patch.object(release, "command", side_effect=reject_certificate), self.assertRaises(ValueError):
            release.verify_asr_runtime(self.app, TEAM)
        self.assertTrue(any("--verify" in argv and "--strict" in argv for argv in self.calls))
        with patch.object(release, "command") as command, self.assertRaises(ValueError):
            release.verify_asr_runtime(self.app, "KLMNOPQRST")
        command.assert_not_called()

    def test_asr_signature_refuses_jit_or_library_validation_exceptions(self):
        def injected_policy(argv, **kwargs):
            if "--display" in argv and "--entitlements" in argv and Path(argv[-1]).name == "whisper-cli":
                return plistlib.dumps({"com.apple.security.cs.disable-library-validation": True}), b""
            return self.signed(argv, **kwargs)
        with patch.object(release, "command", side_effect=injected_policy), self.assertRaises(ValueError):
            release.sign_asr_runtime(self.app, IDENTITY, TEAM, self.policy)
        self.assertEqual(release.verify_asr_runtime(self.app)["runtimeSigning"], "ad_hoc_only")

    def test_asr_final_checks_reject_ad_hoc_fallback_or_changed_archive(self):
        with self.assertRaises(ValueError):
            release.verify_asr_runtime(self.app, TEAM)
        with patch.object(release, "command", side_effect=self.signed):
            release.sign_asr_runtime(self.app, IDENTITY, TEAM, self.policy)
        archive = self.app / "Contents/Resources/asr-runtime/whisper-runtime.tar"
        archive.write_bytes(archive.read_bytes() + b"tampered after signing")
        with patch.object(release, "command") as command, self.assertRaises(ValueError):
            release.verify_asr_runtime(self.app, TEAM)
        command.assert_not_called()


if __name__ == "__main__":
    unittest.main()
