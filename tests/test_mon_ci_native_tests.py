import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import MagicMock, call, patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("mon_ci_native_tests", ROOT / "tools/mon_ci_native_tests.py")
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class NativeCiTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.package = self.root / "app/src-tauri"
        self.package.mkdir(parents=True)
        self.manifest = self.package / "Cargo.toml"
        self.manifest.write_text("[package]\nname = 'mon-desktop'\n", encoding="utf-8")
        self.executable = self.root / "target with spaces/debug/deps/test binary.exe"
        self.executable.parent.mkdir(parents=True)
        self.executable.write_bytes(b"synthetic executable; never launched")
        self.sysroot_lib = self.root / "toolchain/lib"
        self.sysroot_lib.mkdir(parents=True)
        self.environment = {"MON_WINDOWS_MODEL_OPTIONS": "catalog fixture", "TAURI_CONFIG": "fixture config"}
        self.original_runtime_environment = native.runtime_environment
        runtime = patch.object(native, "runtime_environment", return_value=self.environment)
        self.runtime = runtime.start()
        self.addCleanup(runtime.stop)

    def artifact(self, **patches):
        return {"reason": "compiler-artifact", "target": {"name": native.TEST_TARGET},
                "profile": {"test": True}, "executable": str(self.executable), **patches}

    def cargo(self, messages, code=0):
        child = MagicMock()
        child.__enter__.return_value = child
        child.stdout = io.StringIO("".join(json.dumps(message) + "\n" for message in messages))
        child.wait.return_value = code
        return patch.object(native.subprocess, "Popen", return_value=child)

    def test_builds_once_and_runs_both_suites_from_package_directory(self):
        with self.cargo([self.artifact()]) as cargo, \
                patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertEqual(native.run_native_tests(self.root, swift_fixture=True), 0)
        cargo.assert_called_once_with(
            ["cargo", "test", "--manifest-path", str(self.manifest), "--locked", "--lib",
             "--no-run", "--message-format=json"], cwd=self.package, stdout=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace")
        self.assertEqual(run.call_args_list, [
            call([str(self.executable)], cwd=self.package, env=self.environment, check=False),
            call([str(self.executable), native.SWIFT_FIXTURE, "--ignored"],
                 cwd=self.package, env=self.environment, check=False),
        ])
        self.runtime.assert_called_once_with(self.executable, [], self.package)

    def test_normal_suite_runs_once_without_swift_fixture(self):
        with self.cargo([self.artifact()]), \
                patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertEqual(native.run_native_tests(self.root), 0)
        run.assert_called_once_with([str(self.executable)], cwd=self.package, env=self.environment, check=False)

    def test_build_failure_preserves_exit_code_and_never_launches_tests(self):
        with self.cargo([], code=101), patch.object(native.subprocess, "run") as run:
            self.assertEqual(native.run_native_tests(self.root, swift_fixture=True), 101)
        run.assert_not_called()

    def test_missing_or_ambiguous_test_artifacts_never_launch_tests(self):
        for messages in ([], [self.artifact(), self.artifact()]):
            with self.subTest(messages=messages), self.cargo(messages), \
                    patch.object(native.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "expected one"):
                    native.run_native_tests(self.root, swift_fixture=True)
                run.assert_not_called()

    def test_only_matching_library_test_artifact_is_selected(self):
        messages = [self.artifact(target={"name": "other_library"}),
                    self.artifact(profile={"test": False}),
                    self.artifact(target=None), self.artifact(profile=None),
                    self.artifact(executable=None), self.artifact()]
        with self.cargo(messages), \
                patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertEqual(native.run_native_tests(self.root), 0)
        run.assert_called_once_with([str(self.executable)], cwd=self.package, env=self.environment, check=False)

    def test_missing_or_relative_executable_is_rejected(self):
        for executable in ("relative-test.exe", str(self.root / "absent-test.exe")):
            with self.subTest(executable=executable), self.cargo([self.artifact(executable=executable)]), \
                    patch.object(native.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "existing absolute"):
                    native.run_native_tests(self.root)
                run.assert_not_called()

    def test_normal_test_failure_preserves_exit_code_and_skips_ignored_fixture(self):
        with self.cargo([self.artifact()]), \
                patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess([], 17)) as run:
            self.assertEqual(native.run_native_tests(self.root, swift_fixture=True), 17)
        run.assert_called_once_with([str(self.executable)], cwd=self.package, env=self.environment, check=False)

    def test_ignored_fixture_failure_preserves_exit_code(self):
        with self.cargo([self.artifact()]), patch.object(native.subprocess, "run", side_effect=[
                subprocess.CompletedProcess([], 0), subprocess.CompletedProcess([], 9)]) as run:
            self.assertEqual(native.run_native_tests(self.root, swift_fixture=True), 9)
        self.assertEqual(run.call_count, 2)

    def test_compiler_diagnostics_and_non_json_output_remain_visible(self):
        child = MagicMock()
        child.__enter__.return_value = child
        child.stdout = io.StringIO("plain Cargo output\n" + json.dumps({
            "reason": "compiler-message", "message": {"rendered": "error: fixture diagnostic\n"}}) + "\n")
        child.wait.return_value = 101
        output, errors = io.StringIO(), io.StringIO()
        with patch.object(native.subprocess, "Popen", return_value=child), \
                redirect_stdout(output), redirect_stderr(errors):
            self.assertEqual(native.build_library_tests(self.manifest), (101, [], []))
        self.assertIn("plain Cargo output", output.getvalue())
        self.assertIn("error: fixture diagnostic", errors.getvalue())

    def test_unrendered_compiler_diagnostic_is_still_reported(self):
        errors = io.StringIO()
        message = {"reason": "compiler-message", "message": {
            "rendered": None, "message": "fixture diagnostic without rendering"}}
        with self.cargo([message], code=101), redirect_stderr(errors):
            self.assertEqual(native.build_library_tests(self.manifest), (101, [], []))
        self.assertIn("fixture diagnostic without rendering", errors.getvalue())

    def test_build_collects_linked_paths_from_cargo_messages(self):
        paths = ["native=" + str(self.executable.parent.parent / "build/fixture/out")]
        with self.cargo([{"reason": "build-script-executed", "linked_paths": paths}, self.artifact()]):
            self.assertEqual(native.build_library_tests(self.manifest), (0, [self.executable], paths))

    def test_windows_runtime_search_keeps_only_target_link_paths_and_inherited_environment(self):
        target = self.executable.parent.parent.parent
        linked = target / "debug/build/native/out"
        outside = self.root / "external-library"
        sibling = target.with_name(target.name + "-other")
        before = {"PATH": "existing-one;existing-two", "RUSTC": "fixture rustc",
                  "MON_WINDOWS_MODEL_OPTIONS": "catalog fixture"}
        with patch.dict(native.os.environ, before, clear=True), patch.object(native.sys, "platform", "win32"), \
                patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess(
                    [], 0, str(self.sysroot_lib) + "\n")) as query:
            result = self.original_runtime_environment(self.executable,
                ["native=" + str(linked), str(linked), "native=" + str(outside), str(sibling)], self.package)
            self.assertEqual(dict(native.os.environ), before)
        expected = [str(linked), str(self.executable.parent), str(self.executable.parent.parent),
                    str(self.sysroot_lib), before["PATH"]]
        self.assertEqual(result["PATH"], ";".join(expected))
        self.assertEqual(result["MON_WINDOWS_MODEL_OPTIONS"], "catalog fixture")
        query.assert_called_once_with(["fixture rustc", "--print", "target-libdir"],
            cwd=self.package, env=before, stdout=subprocess.PIPE, check=True,
            text=True, encoding="utf-8", errors="replace")

    def test_macos_unset_fallback_includes_default_user_and_system_paths(self):
        home = self.root / "fixture-home"
        with patch.dict(native.os.environ, {"HOME": str(home)}, clear=True), \
                patch.object(native.sys, "platform", "darwin"), \
                patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess(
                    [], 0, str(self.sysroot_lib))):
            result = self.original_runtime_environment(self.executable, [], self.package)
        self.assertEqual(result["DYLD_FALLBACK_LIBRARY_PATH"], ":".join([
            str(self.executable.parent), str(self.executable.parent.parent), str(self.sysroot_lib),
            str(home / "lib"), "/usr/local/lib", "/usr/lib"]))

    def test_existing_macos_and_unix_search_values_are_preserved(self):
        for platform, variable, existing in (("darwin", "DYLD_FALLBACK_LIBRARY_PATH", "/custom/one:/custom/two"),
                                             ("darwin", "DYLD_FALLBACK_LIBRARY_PATH", ""),
                                             ("linux", "LD_LIBRARY_PATH", "/custom/library")):
            with self.subTest(platform=platform, existing=existing), \
                    patch.dict(native.os.environ, {variable: existing}, clear=True), \
                    patch.object(native.sys, "platform", platform), \
                    patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess(
                        [], 0, str(self.sysroot_lib))):
                result = self.original_runtime_environment(self.executable, [], self.package)
            self.assertEqual(result[variable], ":".join([str(self.executable.parent),
                str(self.executable.parent.parent), str(self.sysroot_lib), existing]))

    def test_rustc_query_failure_stops_before_running_any_tests(self):
        errors = io.StringIO()
        with self.cargo([self.artifact()]), patch.object(native, "ROOT", self.root), \
                patch.object(native, "runtime_environment", self.original_runtime_environment), \
                patch.object(native.subprocess, "run", side_effect=subprocess.CalledProcessError(
                    7, ["rustc", "--print", "target-libdir"])) as run, redirect_stderr(errors):
            self.assertEqual(native.main(["--swift-fixture"]), 1)
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.args[0][1:], ["--print", "target-libdir"])
        self.assertIn("exit status 7", errors.getvalue())

    def test_invalid_rustc_library_directory_is_rejected(self):
        for output in ("", "relative/path", str(self.root / "absent-lib"), "one\ntwo"):
            with self.subTest(output=output), patch.object(native.subprocess, "run",
                    return_value=subprocess.CompletedProcess([], 0, output)):
                with self.assertRaisesRegex(ValueError, "existing absolute target library"):
                    self.original_runtime_environment(self.executable, [], self.package)

    def test_cli_reports_build_launch_errors_and_returns_failure(self):
        errors = io.StringIO()
        with patch.object(native, "ROOT", self.root), \
                patch.object(native.subprocess, "Popen", side_effect=FileNotFoundError("cargo unavailable")), \
                redirect_stderr(errors):
            self.assertEqual(native.main(["--swift-fixture"]), 1)
        self.assertIn("cargo unavailable", errors.getvalue())


if __name__ == "__main__":
    unittest.main()
