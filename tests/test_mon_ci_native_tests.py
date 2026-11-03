import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
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
        # Windows may spawn a process to resolve uname on its first call.
        # Keep host diagnostics independent of the mocked Cargo/test processes.
        host = patch.object(native.platform, "uname", return_value=SimpleNamespace(
            system="TestOS", release="test-release", machine="test-machine"))
        host.start()
        self.addCleanup(host.stop)

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

    def extra_artifact(self):
        binary = self.executable.with_name("application tests.exe")
        binary.write_bytes(b"synthetic binary test target")
        return binary, self.artifact(target={"name": "mon-desktop"}, executable=str(binary))

    def test_all_targets_build_preserves_library_binary_and_doc_checks(self):
        binary, message = self.extra_artifact()
        with self.cargo([self.artifact(), message]) as cargo, patch.object(
                native.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertEqual(native.run_native_tests(self.root, all_targets=True), 0)
        self.assertNotIn("--lib", cargo.call_args.args[0])
        self.assertEqual([item.args[0] for item in run.call_args_list], [
            [str(self.executable)], [str(binary)],
            ["cargo", "test", "--manifest-path", str(self.manifest), "--locked", "--doc"],
        ])

    def test_quick_uses_common_classifier_and_keeps_binary_and_doc_targets(self):
        binary, message = self.extra_artifact()
        suite = MagicMock(return_value=0)
        module = SimpleNamespace(run_suite=suite)
        with self.cargo([self.artifact(), message]), patch.dict(
                "sys.modules", {"tools.mon_rust_suites": module}), patch.object(
                native.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertEqual(native.run_native_tests(
                self.root, suite="quick", all_targets=True, threads=3), 0)
        suite.assert_called_once_with(self.executable, cwd=self.package, env=self.environment,
                                      suite="quick", threads=3)
        self.assertEqual(run.call_args_list[0].args[0], [str(binary), "--test-threads=3"])
        self.assertEqual(run.call_args_list[1].args[0][-1], "--doc")

    def test_heavy_uses_common_classifier_without_running_binary_or_docs(self):
        _, message = self.extra_artifact()
        suite = MagicMock(return_value=0)
        with self.cargo([self.artifact(), message]), patch.dict(
                "sys.modules", {"tools.mon_rust_suites": SimpleNamespace(run_suite=suite)}), \
                patch.object(native.subprocess, "run") as run:
            self.assertEqual(native.run_native_tests(self.root, suite="heavy", all_targets=True), 0)
        self.assertEqual(suite.call_args.kwargs["suite"], "heavy")
        run.assert_not_called()

    def test_suite_failure_propagates_and_skips_remaining_targets(self):
        _, message = self.extra_artifact()
        with self.cargo([self.artifact(), message]), patch.dict(
                "sys.modules", {"tools.mon_rust_suites": SimpleNamespace(run_suite=MagicMock(return_value=19))}), \
                patch.object(native.subprocess, "run") as run:
            self.assertEqual(native.run_native_tests(self.root, suite="quick", all_targets=True), 19)
        run.assert_not_called()

    def test_parallel_quick_and_heavy_forward_processes_without_hiding_failure(self):
        for selected, processes in [("quick", 2), ("heavy", 4)]:
            helper = MagicMock(return_value=23)
            with self.subTest(suite=selected, processes=processes), self.cargo([self.artifact()]), \
                    patch.dict("sys.modules", {"tools.mon_rust_suites": SimpleNamespace(run_suite=helper)}), \
                    patch.object(native.subprocess, "run") as run:
                self.assertEqual(native.run_native_tests(
                    self.root, suite=selected, processes=processes, threads=4), 23)
            helper.assert_called_once_with(self.executable, cwd=self.package, env=self.environment,
                                           suite=selected, threads=4, processes=processes)
            run.assert_not_called()

    def test_invalid_or_full_parallel_selection_is_rejected_before_build(self):
        for selected, processes in [("quick", 0), ("heavy", 5), ("full", 2)]:
            with self.subTest(suite=selected, processes=processes), \
                    patch.object(native.subprocess, "Popen") as build:
                with self.assertRaises(ValueError):
                    native.run_native_tests(self.root, suite=selected, processes=processes)
            build.assert_not_called()

    def test_cli_forwards_parallelism_and_reports_measured_process_count(self):
        helper = MagicMock(return_value=0)
        report = self.root / "timings.json"
        with self.cargo([self.artifact()]), patch.object(native, "ROOT", self.root), \
                patch.dict("sys.modules", {"tools.mon_rust_suites": SimpleNamespace(run_suite=helper)}):
            self.assertEqual(native.main(["--suite", "quick", "--processes", "2", "--threads", "4",
                                          "--timings-json", str(report)]), 0)
        self.assertEqual(helper.call_args.kwargs["processes"], 2)
        self.assertEqual(json.loads(report.read_text())["processes"], 2)

    def test_binary_failure_prevents_doc_check_and_preserves_exit_code(self):
        _, message = self.extra_artifact()
        with self.cargo([self.artifact(), message]), patch.object(native.subprocess, "run", side_effect=[
                subprocess.CompletedProcess([], 0), subprocess.CompletedProcess([], 23)]) as run:
            self.assertEqual(native.run_native_tests(self.root, all_targets=True), 23)
        self.assertEqual(run.call_count, 2)

    def test_all_targets_missing_or_duplicated_library_is_rejected(self):
        _, message = self.extra_artifact()
        for messages in ([message], [self.artifact(), self.artifact(), message]):
            with self.subTest(messages=messages), self.cargo(messages), \
                    patch.object(native.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "expected one"):
                    native.run_native_tests(self.root, all_targets=True)
                run.assert_not_called()

    def test_quick_cannot_silently_enable_native_fixture_preparation(self):
        with patch.object(native.subprocess, "Popen") as build:
            with self.assertRaisesRegex(ValueError, "Swift integration fixture"):
                native.run_native_tests(self.root, suite="quick", swift_fixture=True)
        build.assert_not_called()

    def test_timing_report_preserves_failure_and_total_includes_both_phases(self):
        report = self.root / "diagnostics/rust-tests.json"
        with self.cargo([], code=101):
            self.assertEqual(native.run_native_tests(self.root, timings_json=report), 101)
        value = json.loads(report.read_text())
        self.assertEqual(value["suite"], "full")
        self.assertEqual(value["platform"], "TestOS-test-release-test-machine")
        self.assertEqual(value["exitCode"], 101)
        self.assertGreaterEqual(value["totalSeconds"], value["buildSeconds"] + value["executionSeconds"])

    def test_timing_report_is_written_when_build_launch_raises(self):
        report = self.root / "diagnostics/rust-tests.json"
        with patch.object(native.subprocess, "Popen", side_effect=OSError("cannot launch")):
            with self.assertRaisesRegex(OSError, "cannot launch"):
                native.run_native_tests(self.root, timings_json=report)
        self.assertEqual(json.loads(report.read_text())["exitCode"], 1)

    def test_full_threads_are_passed_to_normal_and_explicit_fixture(self):
        with self.cargo([self.artifact()]), patch.object(native.subprocess, "run",
                return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertEqual(native.run_native_tests(self.root, swift_fixture=True, threads=4), 0)
        self.assertTrue(all(item.args[0][-1] == "--test-threads=4" for item in run.call_args_list))


if __name__ == "__main__":
    unittest.main()
