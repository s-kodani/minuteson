"""Frozen-source provenance, partial reports and redundant Rust-setting audit."""
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("mon_ci_benchmark", ROOT / "tools/mon_ci_benchmark.py")
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)
SHA = "a" * 40


class ProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def test_exact_clean_checkout_is_verified_before_any_benchmark(self):
        with patch.object(bench, "query", side_effect=[SHA, ""]) as query:
            self.assertEqual(bench.verify_source(self.root, SHA), SHA)
        self.assertEqual(query.call_args_list[0].args[0], ["git", "-C", str(self.root), "rev-parse", "HEAD"])
        self.assertIn("--untracked-files=no", query.call_args_list[1].args[0])

    def test_bad_sha_wrong_head_and_dirty_tracked_source_are_rejected(self):
        for supplied in ("short", "A" * 40, "a" * 64):
            with self.subTest(supplied=supplied), patch.object(bench, "query") as query:
                with self.assertRaisesRegex(bench.BenchmarkError, "40-character"):
                    bench.verify_source(self.root, supplied)
                query.assert_not_called()
        for output, reason in ((["b" * 40], "HEAD"), ([SHA, " M Cargo.toml"], "tracked source")):
            with self.subTest(reason=reason), patch.object(bench, "query", side_effect=output):
                with self.assertRaisesRegex(bench.BenchmarkError, reason):
                    bench.verify_source(self.root, SHA)

    def test_reports_cannot_overwrite_source_or_follow_linked_parent(self):
        with self.assertRaisesRegex(bench.BenchmarkError, "output/rust-tests"):
            bench.report_path(self.root, Path("app/model-catalog/macos.json"))
        with patch.object(bench, "query", return_value="tracked.json"):
            with self.assertRaisesRegex(bench.BenchmarkError, "tracked source"):
                bench.report_path(self.root, Path("output/rust-tests/tracked.json"))
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(bench.BenchmarkError, "RUNNER_TEMP"):
                bench.report_path(self.root, self.root.parent / "outside.json")

    def test_cache_mutations_require_this_disposable_hosted_workspace(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(bench.BenchmarkError, "disposable GitHub Actions"):
                bench.require_disposable_workspace(self.root)
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
                "GITHUB_ACTIONS": "true", "GITHUB_WORKSPACE": str(self.root), "RUNNER_TEMP": temporary}):
            bench.require_disposable_workspace(self.root)
            with patch.dict(os.environ, {"GITHUB_WORKSPACE": str(self.root / "different")}):
                with self.assertRaisesRegex(bench.BenchmarkError, "GITHUB_WORKSPACE"):
                    bench.require_disposable_workspace(self.root)


class RustAuditTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        (self.root / "Cargo.toml").write_text('[profile.test]\nincremental = true\n[profile.test.package.mon-desktop]\nopt-level = 1\n')

    def test_actual_ci_zero_makes_proposal_redundant_without_running_or_cleaning_anything(self):
        report, checkpoint, run = {}, Mock(), Mock()
        with patch.dict(os.environ, {"CARGO_INCREMENTAL": "0"}):
            before = dict(os.environ)
            bench.rust_benchmark(self.root, report, checkpoint, run)
            self.assertEqual(dict(os.environ), before)
        audit = report["rust"]
        self.assertFalse(audit["eligible"])
        self.assertEqual(audit["status"], "redundant")
        self.assertTrue(audit["sourceProfileIncremental"])
        self.assertFalse(audit["effectiveCurrentIncremental"])
        self.assertFalse(audit["effectiveCandidateIncremental"])
        self.assertEqual(audit["trials"], [])
        run.assert_not_called()
        self.assertEqual(checkpoint.call_count, 2)

    def test_missing_or_different_inherited_setting_fails_instead_of_faking_a_baseline(self):
        for value in (None, "", "1", "false"):
            report, run = {}, Mock()
            with self.subTest(value=value), patch.dict(os.environ, {}, clear=True):
                if value is not None:
                    os.environ["CARGO_INCREMENTAL"] = value
                with self.assertRaisesRegex(bench.BenchmarkError, "actual inherited CARGO_INCREMENTAL=0"):
                    bench.rust_benchmark(self.root, report, Mock(), run)
            self.assertEqual(report["rust"]["status"], "inconclusive")
            self.assertFalse(report["rust"]["eligible"])
            run.assert_not_called()

    def test_main_records_actual_source_separately_from_workflow_source(self):
        report_path = self.root / "output/rust-tests/rust.json"
        with patch.object(bench, "verify_source", return_value=SHA), patch.object(bench, "query", return_value=""), \
                patch.object(bench, "context", return_value={"toolchain": {"rustc": "fixture"}}), \
                patch.dict(os.environ, {"CARGO_INCREMENTAL": "0", "GITHUB_SHA": "b" * 40}):
            self.assertEqual(bench.main(["--root", str(self.root), "--source-sha", SHA, "--mode", "rust",
                                         "--report", str(report_path)]), 0)
        result = json.loads(report_path.read_text())
        self.assertEqual(result["sourceSha"], SHA)
        self.assertEqual(result["workflowSha"], "b" * 40)
        self.assertTrue(result["completed"])
        self.assertEqual(result["exitCode"], 0)

    def test_main_writes_nonzero_partial_report_when_audit_is_inconclusive(self):
        report_path = self.root / "output/rust-tests/rust.json"
        with patch.object(bench, "verify_source", return_value=SHA), patch.object(bench, "query", return_value=""), \
                patch.object(bench, "context", return_value={}), patch.dict(os.environ, {}, clear=True), \
                redirect_stderr(io.StringIO()):
            self.assertEqual(bench.main(["--root", str(self.root), "--source-sha", SHA, "--mode", "rust",
                                         "--report", str(report_path)]), 1)
        result = json.loads(report_path.read_text())
        self.assertFalse(result["completed"])
        self.assertEqual(result["rust"]["status"], "inconclusive")
        self.assertIn("CARGO_INCREMENTAL=0", result["error"])

    def test_wrong_source_writes_partial_report_before_any_measurement_or_cleanup(self):
        report_path = self.root / "output/rust-tests/wrong-source.json"
        with patch.object(bench, "verify_source", side_effect=bench.BenchmarkError("wrong HEAD")), \
                patch.object(bench, "query", return_value=""), patch.object(bench, "context") as context, \
                patch.object(bench, "cache_benchmark") as cache, redirect_stderr(io.StringIO()):
            self.assertEqual(bench.main(["--root", str(self.root), "--source-sha", SHA, "--mode", "cache",
                                         "--report", str(report_path)]), 1)
        result = json.loads(report_path.read_text())
        self.assertIsNone(result["sourceSha"])
        self.assertEqual(result["error"], "wrong HEAD")
        context.assert_not_called()
        cache.assert_not_called()

    def test_failing_cache_trial_retains_partial_results_and_original_exit_status(self):
        report_path = self.root / "output/rust-tests/partial.json"
        def fail(root, report, checkpoint, run):
            report["cache"] = {"trials": [{"trial": "cold", "status": "failure"}]}
            checkpoint()
            raise subprocess.CalledProcessError(23, ["synthetic preparation"])
        with patch.object(bench, "verify_source", return_value=SHA), patch.object(bench, "query", return_value=""), \
                patch.object(bench, "context", return_value={}), patch.object(bench, "require_disposable_workspace"), \
                patch.object(bench, "cache_benchmark", side_effect=fail), redirect_stderr(io.StringIO()):
            self.assertEqual(bench.main(["--root", str(self.root), "--source-sha", SHA, "--mode", "cache",
                                         "--report", str(report_path)]), 23)
        result = json.loads(report_path.read_text())
        self.assertEqual(result["cache"]["trials"][0]["status"], "failure")
        self.assertTrue(result["sourceUnchanged"])
        self.assertFalse(result["completed"])

    def test_posttrial_source_mutation_invalidates_an_otherwise_successful_result(self):
        report_path = self.root / "output/rust-tests/changed-source.json"
        with patch.object(bench, "verify_source", side_effect=[SHA, bench.BenchmarkError("tracked source changed")]), \
                patch.object(bench, "query", return_value=""), patch.object(bench, "context", return_value={}), \
                patch.dict(os.environ, {"CARGO_INCREMENTAL": "0"}):
            self.assertEqual(bench.main(["--root", str(self.root), "--source-sha", SHA, "--mode", "rust",
                                         "--report", str(report_path)]), 1)
        result = json.loads(report_path.read_text())
        self.assertFalse(result["sourceUnchanged"])
        self.assertFalse(result["completed"])
        self.assertEqual(result["error"], "tracked source changed")

    def test_context_is_allowlisted_not_a_dump_of_environment(self):
        with patch.dict(os.environ, {"GITHUB_TOKEN": "secret-token", "PRIVATE_OTHER": "secret-other",
                                    "CARGO_INCREMENTAL": "0"}, clear=True), \
                patch.object(bench, "query", return_value="fixture toolchain"):
            result = bench.context(self.root)
        self.assertEqual(result["environment"], {"CARGO_INCREMENTAL": "0"})
        self.assertNotIn("secret", json.dumps(result))


if __name__ == "__main__":
    unittest.main()
