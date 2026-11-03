"""Cross-platform suite contracts, plus real POSIX subprocess selection checks."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("mon_rust_suites", ROOT / "tools/mon_rust_suites.py")
suites = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(suites)


class SuiteDefinitionTests(unittest.TestCase):
    def test_committed_rules_keep_basic_backup_voice_and_cleanup_in_quick(self):
        essential = [
            "storage::backup::archive::tests::current_contents_roundtrip_has_one_baseline_and_no_jobs_or_edits",
            "storage::backup::archive::tests::format8_reader_rejects_corruption_truncation_extra_bytes_and_old_versions",
            "storage::backup::voice_audio_tests::owned_voice_audio_roundtrips_without_its_deleted_source_meeting",
            "storage::audio_publication::cleanup_tests::completed_workspace_retires_unknown_audio_and_parts_but_preserves_documents_and_caches",
        ]
        self.assertEqual(suites.select_tests(essential, "quick"), essential)
        native = ["windows_integration_tests::windows_clock_drift_uses_the_same_committed_raw_maps_and_finalizes",
                  "capture_test::files::windows_five_second_synthetic_capture_validates_and_releases_audio"]
        self.assertEqual(suites.select_tests(native, "heavy"), native)

    def test_partition_is_disjoint_complete_and_new_tests_default_to_quick(self):
        names = ["regular::first", "slow::first", "regular::new", "other::special"]
        rules = [{"prefix": "slow::", "reason": "repeated crash boundaries"},
                 {"name": "other::special", "reason": "large input"}]
        quick = suites.select_tests(names, "quick", rules)
        heavy = suites.select_tests(names, "heavy", rules)
        self.assertEqual(quick, ["regular::first", "regular::new"])
        self.assertEqual(heavy, ["slow::first", "other::special"])
        self.assertFalse(set(quick) & set(heavy))
        self.assertEqual(sorted(quick + heavy), sorted(names))
        self.assertEqual(suites.select_tests(names, "full", rules), names)

    def test_rule_prefixes_are_anchored_and_exact_names_do_not_match_suffixes(self):
        names = ["slow::test", "outer::slow::test", "slowly::test", "other::special_longer"]
        rules = [{"prefix": "slow::", "reason": "integration"},
                 {"name": "other::special", "reason": "large"}]
        self.assertEqual(suites.select_tests(names, "heavy", rules), ["slow::test"])

    def test_unknown_suite_and_duplicate_inventory_are_rejected(self):
        with self.assertRaises(ValueError):
            suites.select_tests([], "unknown", [])
        with self.assertRaises(ValueError):
            suites.select_tests(["same::test", "same::test"], "full", [])

    def test_invalid_rule_definitions_cannot_silently_disable_tests(self):
        definitions = [
            {"schemaVersion": 2, "heavy": []},
            {"schemaVersion": 1, "heavy": {}},
            {"schemaVersion": 1, "heavy": [{"prefix": "slow::", "reason": " "}]},
            {"schemaVersion": 1, "heavy": [{"prefix": "slow", "reason": "ambiguous"}]},
            {"schemaVersion": 1, "heavy": [{"name": "slow::", "reason": "not exact"}]},
            {"schemaVersion": 1, "heavy": [{"prefix": "slow::", "name": "slow::test", "reason": "both"}]},
            {"schemaVersion": 1, "heavy": [{"name": "slow::test", "reason": "one"},
                                             {"name": "slow::test", "reason": "duplicate"}]},
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "rules.json"
            for definition in definitions:
                with self.subTest(definition=definition):
                    path.write_text(json.dumps(definition))
                    with self.assertRaises(ValueError):
                        suites.heavy_rules(path)

    def test_inventory_rejects_missing_summary_duplicates_and_benchmarks(self):
        for output in ("a: test", "a: test\n0 tests, 0 benchmarks\n",
                       "a: test\na: test\n2 tests, 0 benchmarks\n", "0 tests, 1 benchmark\n"):
            with self.subTest(output=output), self.assertRaises(ValueError):
                suites.inventory(output)
        self.assertEqual(suites.inventory("0 tests, 0 benchmarks\n"), [])

    def test_exact_batches_bound_utf16_argv_and_preserve_every_test_once(self):
        names = ["module::" + "\U0001f600" * 30 + str(i) for i in range(150)]
        groups = list(suites.exact_batches(names, maximum=1000))
        self.assertEqual([name for group in groups for name in group], names)
        self.assertTrue(all(len(group) <= 64 for group in groups))
        self.assertTrue(all(sum(len(name.encode("utf-16-le")) // 2 + 3 for name in group) <= 1000
                            for group in groups))
        with self.assertRaises(ValueError):
            list(suites.exact_batches(["x" * 101], maximum=100))


FAKE_LIBTEST = r'''
import json, os, sys
names = json.loads(os.environ["FAKE_NAMES"])
ignored = set(json.loads(os.environ.get("FAKE_IGNORED", "[]")))
args = sys.argv[1:]
filters, skips = [], []
i = 0
while i < len(args):
    arg = args[i]
    if arg == "--skip":
        skips.append(args[i+1]); i += 2; continue
    if not arg.startswith("-"):
        filters.append(arg)
    i += 1
def matches(name, term):
    return name == term if "--exact" in args else term in name
selected = [name for name in names if (not filters or any(matches(name, f) for f in filters))
            and not any(matches(name, s) for s in skips)]
if "--ignored" in args:
    selected = [name for name in selected if name in ignored]
if "--list" in args:
    for name in selected:
        print(name + ": test")
    print(str(len(selected)) + " tests, 0 benchmarks")
    sys.exit(0)
if "--ignored" not in args and "--include-ignored" not in args:
    selected = [name for name in selected if name not in ignored]
with open(os.environ["FAKE_LOG"], "a") as output:
    output.write(json.dumps({"tests": selected, "args": args}) + "\n")
sys.exit(23 if os.environ.get("FAKE_FAIL") in selected else 0)
'''


@unittest.skipIf(os.name == "nt", "POSIX executable script fixture; Windows argv covered above")
class SuiteSubprocessTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.executable = self.root / "fake libtest"
        self.executable.write_text("#!" + sys.executable + "\n" + FAKE_LIBTEST)
        self.executable.chmod(0o700)
        self.log = self.root / "runs.jsonl"
        self.environment = {**os.environ, "FAKE_NAMES": json.dumps(
            ["regular::first", "regular::second", "slow::boundary", "slow::child"]),
            "FAKE_IGNORED": json.dumps(["slow::child"]), "FAKE_LOG": str(self.log)}
        rules = patch.object(suites, "heavy_rules", return_value=[
            {"prefix": "slow::", "reason": "fixture integration"}])
        rules.start()
        self.addCleanup(rules.stop)

    def run_suite(self, suite, **kwargs):
        return suites.run_suite(self.executable, cwd=self.root, env=self.environment,
                                suite=suite, **kwargs)

    def runs(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_quick_preserves_single_scheduler_and_omits_only_declared_heavy_tests(self):
        self.assertEqual(self.run_suite("quick", threads=3), 0)
        self.assertEqual(len(self.runs()), 1)
        self.assertEqual(self.runs()[0]["tests"], ["regular::first", "regular::second"])
        self.assertIn("--test-threads=3", self.runs()[0]["args"])

    def test_heavy_exact_selection_does_not_automatically_run_ignored_children(self):
        self.assertEqual(self.run_suite("heavy"), 0)
        self.assertEqual(self.runs()[0]["tests"], ["slow::boundary"])
        self.assertIn("--exact", self.runs()[0]["args"])
        self.assertNotIn("--ignored", self.runs()[0]["args"])

    def test_explicit_ignored_child_filter_is_preserved(self):
        self.assertEqual(self.run_suite("heavy", extra=["slow::child", "--ignored"]), 0)
        self.assertEqual(self.runs()[0]["tests"], ["slow::child"])

    def test_exact_quick_filter_keeps_declared_partition(self):
        self.assertEqual(self.run_suite("quick", extra=["regular::first", "--exact"]), 0)
        self.assertEqual(self.runs()[0]["tests"], ["regular::first"])

    def test_heavy_respects_skip_filter_when_materializing_exact_batches(self):
        self.environment["FAKE_NAMES"] = json.dumps(["slow::first", "slow::second"])
        self.environment["FAKE_IGNORED"] = "[]"
        self.assertEqual(self.run_suite("heavy", extra=["--skip", "first"]), 0)
        self.assertEqual(self.runs()[0]["tests"], ["slow::second"])

    def test_heavy_failure_stops_before_following_batches(self):
        self.environment["FAKE_NAMES"] = json.dumps([f"slow::test_{i}" for i in range(130)])
        self.environment["FAKE_IGNORED"] = "[]"
        self.environment["FAKE_FAIL"] = "slow::test_0"
        self.assertEqual(self.run_suite("heavy"), 23)
        self.assertEqual(len(self.runs()), 1)

    def test_parallel_suite_runs_each_selected_test_exactly_once(self):
        regular = [f"regular::test_{i}" for i in range(130)]
        heavy = [f"slow::test_{i}" for i in range(130)]
        self.environment["FAKE_NAMES"] = json.dumps(regular + heavy + ["slow::child"])
        for suite, expected in [("quick", regular), ("heavy", heavy)]:
            with self.subTest(suite=suite):
                self.log.unlink(missing_ok=True)
                self.assertEqual(self.run_suite(suite, processes=4, threads=2), 0)
                executed = [name for run in self.runs() for name in run["tests"]]
                self.assertEqual(sorted(executed), sorted(expected))
                self.assertEqual(len(executed), len(set(executed)))
                self.assertGreater(len(self.runs()), 1)
                self.assertTrue(all("--exact" in run["args"] for run in self.runs()))

    def test_parallel_failure_preserves_nonzero_exit_without_retries(self):
        self.environment["FAKE_NAMES"] = json.dumps([f"regular::test_{i}" for i in range(130)])
        self.environment["FAKE_IGNORED"] = "[]"
        self.environment["FAKE_FAIL"] = "regular::test_0"
        self.assertEqual(self.run_suite("quick", processes=4, threads=2), 23)
        executed = [name for run in self.runs() for name in run["tests"]]
        self.assertEqual(executed.count("regular::test_0"), 1)
        self.assertEqual(len(executed), len(set(executed)))

    def test_invalid_process_count_is_rejected_before_launch(self):
        for processes in (0, 5):
            with self.subTest(processes=processes), self.assertRaises(ValueError):
                self.run_suite("quick", processes=processes)
        self.assertEqual(self.runs(), [])

    def test_full_keeps_normal_ignored_policy_and_runs_once(self):
        self.assertEqual(self.run_suite("full"), 0)
        self.assertEqual(len(self.runs()), 1)
        self.assertEqual(self.runs()[0]["tests"], ["regular::first", "regular::second", "slow::boundary"])
        self.assertEqual(self.runs()[0]["args"], [])

    def test_failure_is_not_retried_and_preserves_exit_code(self):
        self.environment["FAKE_FAIL"] = "regular::first"
        self.assertEqual(self.run_suite("quick"), 23)
        self.assertEqual(len(self.runs()), 1)

    def test_focus_filter_with_no_matching_test_cannot_report_success(self):
        self.assertNotEqual(self.run_suite("quick", extra=["missing::test"]), 0)
        self.assertEqual(self.runs(), [])

    def test_focus_filter_selecting_only_other_suite_cannot_report_success(self):
        self.assertNotEqual(self.run_suite("quick", extra=["slow::boundary"]), 0)
        self.assertEqual(self.runs(), [])

    def test_unfiltered_target_without_tests_is_valid(self):
        self.environment["FAKE_NAMES"] = "[]"
        self.assertEqual(self.run_suite("quick"), 0)

    def test_anchored_rule_and_libtest_substring_collision_fails_before_execution(self):
        self.environment["FAKE_NAMES"] = json.dumps(["regular::first", "outer::slow::boundary"])
        with self.assertRaisesRegex(ValueError, "skip selection differs"):
            self.run_suite("quick")
        self.assertEqual(self.runs(), [])

    def test_control_flags_cannot_turn_execution_into_a_silent_check(self):
        for flag in ("--list", "--help", "-h"):
            with self.subTest(flag=flag), self.assertRaises(ValueError):
                self.run_suite("quick", extra=[flag])
        self.assertEqual(self.runs(), [])

    def test_diagnostic_report_names_exact_selected_partition(self):
        report = self.root / "diagnostics/result.json"
        self.assertEqual(self.run_suite("quick", report_path=report), 0)
        result = json.loads(report.read_text())
        self.assertEqual(result["selected"], ["regular::first", "regular::second"])
        self.assertEqual(result["inventoried"], 4)
        self.assertEqual(result["exitCode"], 0)
        self.assertGreaterEqual(result["seconds"], 0)


@unittest.skipUnless(shutil.which("rustc"), "standalone Rust fixture requires rustc")
class RealLibtestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.root = Path(cls.temporary.name)
        cls.executable = cls.root / ("fixture.exe" if os.name == "nt" else "fixture")
        source = cls.root / "fixture.rs"
        source.write_text(r'''
fn record(name: &str) {
    use std::io::Write;
    let path = std::env::var_os("MON_SUITE_RUST_LOG").unwrap();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    file.write_all(format!("{name}\n").as_bytes()).unwrap();
}
mod regular {
    #[test] fn first() { super::record("regular::first"); }
    #[test] fn second() { super::record("regular::second"); }
}
mod slow {
    #[test] fn boundary() { super::record("slow::boundary"); }
    #[test] #[ignore] fn child() { super::record("slow::child"); }
}
''')
        subprocess.run(["rustc", "--test", str(source), "-o", str(cls.executable)],
                       check=True, capture_output=True, text=True)

    def test_actual_libtest_preserves_partition_and_explicit_ignored_child(self):
        log = self.root / "run.log"
        environment = {**os.environ, "MON_SUITE_RUST_LOG": str(log)}
        cases = [
            ("quick", [], {"regular::first", "regular::second"}),
            ("heavy", [], {"slow::boundary"}),
            ("full", [], {"regular::first", "regular::second", "slow::boundary"}),
            ("quick", ["regular::first", "--exact"], {"regular::first"}),
            ("heavy", ["slow::child", "--ignored"], {"slow::child"}),
        ]
        with patch.object(suites, "heavy_rules", return_value=[
                {"prefix": "slow::", "reason": "standalone test integration"}]):
            for suite, extra, expected in cases:
                with self.subTest(suite=suite, extra=extra):
                    log.unlink(missing_ok=True)
                    self.assertEqual(suites.run_suite(self.executable, cwd=self.root, env=environment,
                                                     suite=suite, extra=extra, threads=2), 0)
                    executed = log.read_text().splitlines()
                    self.assertEqual(set(executed), expected)
                    self.assertEqual(len(executed), len(expected))


if __name__ == "__main__":
    unittest.main()
