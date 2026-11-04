"""Same-job preparation reuse must never bypass changed inputs or outputs."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import mon_windows_preparation as preparation


class WindowsPreparationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name).resolve()
        self.root = self.directory / "workspace"
        self.root.mkdir()
        self.runner = self.directory / "runner"
        self.runner.mkdir()
        self.receipt = self.runner / "preparation.json"
        self.source = self.root / "source.txt"
        self.source.write_text("original source")
        environment = patch.dict(os.environ, {
            "GITHUB_ACTIONS": "true", "GITHUB_RUN_ID": "42", "GITHUB_RUN_ATTEMPT": "1",
            "GITHUB_JOB": "native", "RUNNER_TEMP": str(self.runner), "MON_WINDOWS_RELEASE_SIGNING": "0",
        })
        environment.start()
        self.addCleanup(environment.stop)
        source = patch.object(preparation.evidence, "source_identity", side_effect=lambda root: {"source": (root / "source.txt").read_text()})
        source.start()
        self.addCleanup(source.stop)
        toolchain = patch.object(preparation, "toolchain", return_value={"compiler": "fixed compiler", "sdk": "fixed SDK"})
        self.toolchain = toolchain.start()
        self.addCleanup(toolchain.stop)
        for name in preparation.REQUIRED:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(("prepared " + name).encode())
        self.compiler = self.directory / "cl.exe"
        self.compiler.write_bytes(b"inert compiler fixture; never executed")
        for name in ("CMakeCache.txt", "CMakeFiles/3.31.0/CMakeCXXCompiler.cmake"):
            path = self.root / "probes/windows-capture/build" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('set(CMAKE_CXX_COMPILER "' + self.compiler.as_posix() + '")\n')

    def ready(self):
        preparation.begin(self.root, self.receipt)
        preparation.record(self.root, self.receipt)

    def test_unchanged_unsigned_preparation_can_be_reused_by_qa_and_store(self):
        self.ready()
        preparation.verify(self.root, self.receipt)
        preparation.verify(self.root, self.receipt)
        value = json.loads(self.receipt.read_text())
        self.assertEqual(value["status"], "ready")
        self.assertEqual(value["identity"]["signing"], "unsigned")
        self.assertEqual(value["identity"]["session"]["workspace"], str(self.root))
        self.assertEqual({row["path"] for row in value["outputs"]}, set(preparation.REQUIRED))

    def test_source_change_during_preparation_never_produces_a_ready_receipt(self):
        preparation.begin(self.root, self.receipt)
        self.source.write_text("changed source")
        with self.assertRaisesRegex(ValueError, "does not match"):
            preparation.record(self.root, self.receipt)
        self.assertEqual(json.loads(self.receipt.read_text())["status"], "preparing")

    def test_incomplete_preparation_or_missing_receipt_cannot_be_reused(self):
        with self.assertRaises(FileNotFoundError):
            preparation.verify(self.root, self.receipt)
        preparation.begin(self.root, self.receipt)
        with self.assertRaisesRegex(ValueError, "does not match"):
            preparation.verify(self.root, self.receipt)
        (self.root / ".cache/windows-runtime/package/runtime.tar").unlink()
        with self.assertRaises(ValueError):
            preparation.record(self.root, self.receipt)
        self.assertEqual(json.loads(self.receipt.read_text())["status"], "preparing")

    def test_changed_output_missing_output_and_extra_payload_are_rejected(self):
        self.ready()
        output = self.root / "app/dist/index.html"
        original = output.read_bytes()
        output.write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "outputs changed"):
            preparation.verify(self.root, self.receipt)
        output.unlink()
        with self.assertRaises(ValueError):
            preparation.verify(self.root, self.receipt)
        output.write_bytes(original)
        (self.root / ".cache/windows-runtime/package/unknown.dll").write_bytes(b"unexpected payload")
        with self.assertRaisesRegex(ValueError, "outputs changed"):
            preparation.verify(self.root, self.receipt)

    def test_source_toolchain_and_capture_configuration_changes_reject_reuse(self):
        self.ready()
        self.source.write_text("different checkout")
        with self.assertRaisesRegex(ValueError, "does not match"):
            preparation.verify(self.root, self.receipt)
        self.source.write_text("original source")
        self.toolchain.return_value = {"compiler": "different compiler"}
        with self.assertRaisesRegex(ValueError, "does not match"):
            preparation.verify(self.root, self.receipt)
        self.toolchain.return_value = {"compiler": "fixed compiler", "sdk": "fixed SDK"}
        (self.root / "probes/windows-capture/build/CMakeCache.txt").write_text("different compiler/SDK")
        with self.assertRaisesRegex(ValueError, "outputs changed"):
            preparation.verify(self.root, self.receipt)

    def test_other_run_attempt_job_workspace_or_signed_build_cannot_reuse(self):
        self.ready()
        for key, value in (("GITHUB_RUN_ID", "43"), ("GITHUB_RUN_ATTEMPT", "2"),
                           ("GITHUB_JOB", "store"), ("MON_WINDOWS_RELEASE_SIGNING", "1")):
            with self.subTest(key=key), patch.dict(os.environ, {key: value}):
                with self.assertRaises(ValueError):
                    preparation.verify(self.root, self.receipt)
        elsewhere = self.directory / "different-workspace"
        elsewhere.mkdir()
        (elsewhere / "source.txt").write_text("original source")
        with self.assertRaisesRegex(ValueError, "does not match"):
            preparation.verify(elsewhere, self.receipt)

    def test_changed_compiler_binary_and_malformed_receipt_are_rejected(self):
        self.ready()
        self.compiler.write_bytes(b"different compiler binary")
        with self.assertRaisesRegex(ValueError, "outputs changed"):
            preparation.verify(self.root, self.receipt)
        self.receipt.write_text('[]')
        with self.assertRaisesRegex(ValueError, "does not match"):
            preparation.verify(self.root, self.receipt)

    def test_receipt_requires_actions_and_runner_temp_and_cannot_overwrite_previous(self):
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}):
            with self.assertRaisesRegex(ValueError, "Actions job"):
                preparation.begin(self.root, self.receipt)
        with self.assertRaisesRegex(ValueError, "runner's temporary"):
            preparation.begin(self.root, self.root / "receipt.json")
        with self.assertRaisesRegex(ValueError, "absolute path"):
            preparation.begin(self.root, Path("relative-receipt.json"))
        preparation.begin(self.root, self.receipt)
        with self.assertRaisesRegex(ValueError, "new preparation"):
            preparation.begin(self.root, self.receipt)

    def test_output_symlink_cannot_be_accepted(self):
        self.ready()
        output = self.root / "app/dist/index.html"
        original = output.read_bytes()
        target = self.root / "other.html"
        target.write_bytes(original)
        output.unlink()
        try:
            output.symlink_to(target)
        except OSError:
            self.skipTest("Creating symlinks is not available")
        with self.assertRaises(ValueError):
            preparation.verify(self.root, self.receipt)

    def test_phase_timings_retain_success_failure_and_launch_errors(self):
        report = self.directory / "phases.json"
        for phase, code in (("preparation", 0), ("qa-packaging", 17)):
            with patch.object(preparation.subprocess, "run", return_value=subprocess.CompletedProcess([], code)):
                self.assertEqual(preparation.run_timed(report, phase, ["synthetic command"]), code)
        with patch.object(preparation.subprocess, "run", side_effect=FileNotFoundError("missing tool")):
            with self.assertRaises(FileNotFoundError):
                preparation.run_timed(report, "store-packaging", ["missing"])
        rows = json.loads(report.read_text())["phases"]
        self.assertEqual([row["status"] for row in rows], ["success", "failure", "failure"])
        self.assertEqual([row["exitCode"] for row in rows], [0, 17, 1])
        self.assertEqual(rows[-1]["failureType"], "FileNotFoundError")
        self.assertTrue(all(row["seconds"] >= 0 for row in rows))
        self.assertEqual(list(self.directory.glob("*.pending")), [])


if __name__ == "__main__":
    unittest.main()
