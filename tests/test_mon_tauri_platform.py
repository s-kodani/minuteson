"""Cross-platform packaging and IPC boundary contracts, without a running app."""
import copy
import json
from pathlib import Path
import re
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]
TAURI = ROOT / "app/src-tauri"


def configuration(name):
    return json.loads((TAURI / name).read_text(encoding="utf8"))


def merge(base, patch):
    """RFC 7396, as used by Tauri: arrays replace, null removes."""
    if not isinstance(patch, dict):
        return copy.deepcopy(patch)
    result = copy.deepcopy(base) if isinstance(base, dict) else {}
    for key, value in patch.items():
        if value is None:
            result.pop(key, None)
        else:
            result[key] = merge(result.get(key), value)
    return result


class TauriPlatformTests(unittest.TestCase):
    def setUp(self):
        self.base = configuration("tauri.conf.json")
        self.permissions = {
            item["identifier"]: set(item["commands"]["allow"])
            for item in tomllib.loads((TAURI / "permissions/application.toml").read_text())["permission"]
        }

    def grants(self, window):
        grants = set()
        enabled = self.base["app"]["security"]["capabilities"]
        for name in enabled:
            capability = configuration(f"capabilities/{name}.json")
            if window in capability["windows"]:
                for permission in capability["permissions"]:
                    grants.update(self.permissions.get(permission, set()))
        return grants

    def test_every_registered_command_is_in_the_acl_manifest_and_has_an_explicit_role(self):
        source = (TAURI / "src/lib.rs").read_text()
        handler = source.split("tauri::generate_handler![", 1)[1].split("];", 1)[0]
        commands = re.findall(r"^\s*(?:\w+::)*(\w+),\s*$", handler, re.M)
        manifest = re.findall(r'"(\w+)"', (TAURI / "app_commands.rs").read_text())
        self.assertGreater(len(commands), 100)
        self.assertEqual(len(commands), len(set(commands)))
        self.assertEqual(commands, manifest)
        self.assertEqual(set(commands), set.union(*self.permissions.values()))
        build = (TAURI / "build.rs").read_text()
        self.assertIn("AppManifest::new().commands(app_commands::COMMANDS)", build)

    def test_auxiliary_windows_cannot_modify_meetings_models_or_saved_settings(self):
        sensitive = {
            "start_recording", "start_audio_import", "start_data_operation",
            "review_model_body_deletion", "apply_model_profile_application",
            "start_model_download", "save_transcript_draft", "save_minutes_markdown",
            "save_ui_preferences", "get_internal_logs", "finish_main_close",
        }
        for window in ["recorder", "recorder-info", "recording-confirmation"]:
            with self.subTest(window=window):
                self.assertFalse(self.grants(window) & sensitive)
                self.assertIn("get_ui_preferences", self.grants(window))
        self.assertTrue(sensitive <= self.grants("main"))
        self.assertFalse(self.grants("unknown"))
        self.assertEqual(self.grants("recording-confirmation"), {
            "get_ui_preferences", "get_recording_confirmation", "decide_recording_confirmation",
        })
        self.assertEqual(self.grants("recorder-info"), {
            "get_ui_preferences", "get_recorder_operation_notices", "close_recorder_info",
        })
        self.assertTrue({"request_recording_confirmation", "get_recording_input_status",
                         "open_main_window", "hide_recorder_panel"} <= self.grants("recorder"))

    def test_capabilities_do_not_grant_generic_shell_file_or_remote_access(self):
        for name in self.base["app"]["security"]["capabilities"]:
            capability = configuration(f"capabilities/{name}.json")
            self.assertNotIn("remote", capability)
            self.assertEqual(capability["windows"], [name])
            self.assertFalse(any(p.startswith(("shell:", "fs:", "dialog:"))
                                 for p in capability["permissions"]))

    def test_windows_qa_merge_keeps_its_identity_windows_and_only_windows_resources(self):
        windows = merge(self.base, configuration("tauri.windows.conf.json"))
        windows = merge(windows, configuration("tauri.windows.bundle.conf.json"))
        qa = merge(windows, configuration("tauri.qa.conf.json"))
        self.assertEqual(qa["identifier"], "jp.mon.desktop.qa")
        self.assertNotEqual(qa["identifier"], windows["identifier"])
        self.assertEqual(qa["bundle"]["targets"], ["nsis"])
        self.assertEqual(qa["bundle"]["windows"]["nsis"]["installMode"], "currentUser")
        self.assertEqual(qa["bundle"]["windows"]["webviewInstallMode"]["type"], "offlineInstaller")
        self.assertFalse(qa["bundle"]["windows"]["allowDowngrades"])
        self.assertEqual({w["label"] for w in qa["app"]["windows"]},
                         {"main", "recorder", "recorder-info", "recording-confirmation"})
        self.assertEqual(qa["app"]["security"], self.base["app"]["security"])
        for window in qa["app"]["windows"]:
            if window["label"] != "main":
                self.assertFalse(window["visible"])
                self.assertTrue(window["skipTaskbar"])
        self.assertNotIn(".app", json.dumps(qa["bundle"]["resources"]))
        self.assertNotIn("build-macos", qa["build"]["beforeBuildCommand"])
        self.assertEqual(qa["bundle"]["externalBin"], ["binaries/mon-windows-capture"])

    def test_macos_configuration_does_not_acquire_windows_build_or_bundle_settings(self):
        mac = merge(self.base, configuration("tauri.macos.conf.json"))
        self.assertEqual(mac["bundle"]["targets"], ["app"])
        self.assertEqual(mac["build"]["beforeBuildCommand"], "sh build-macos.sh")
        self.assertNotIn("windows", mac["bundle"])
        self.assertNotIn("externalBin", mac["bundle"])
        self.assertEqual(mac["app"]["security"], self.base["app"]["security"])


if __name__ == "__main__":
    unittest.main()
