import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

if os.name == "nt":
    raise unittest.SkipTest("The shared Rust test-cache runner requires POSIX file locks and descriptor-relative I/O")

SPEC = importlib.util.spec_from_file_location(
    "mon_rust_tests", Path(__file__).resolve().parents[1] / "tools" / "mon_rust_tests.py")
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class RustRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.repository = self.root / "repo"
        self.repository.mkdir()
        self.cache = runner.Cache(self.repository / ".cache" / "tests", self.repository)

    def test_inventory_and_exact_partition_cover_every_test_once(self):
        names = [f"module{i % 17}::test_{i}" for i in range(1730)]
        output = "\n".join(f"{name}: test" for name in names) + "\n1730 tests, 0 benchmarks\n"
        self.assertEqual(runner.inventory(output), names)
        groups = runner.batches(names, 512)
        self.assertEqual(sorted(name for group in groups for name in group), sorted(names))
        self.assertTrue(all(sum(len(name.encode())+1 for name in group) <= 512 for group in groups))

    def test_inventory_rejects_unknown_missing_duplicate_and_benchmarks(self):
        for value in ["a: test\n", "a: test\n0 tests, 0 benchmarks",
                      "a: test\na: test\n2 tests, 0 benchmarks", "0 tests, 1 benchmark"]:
            with self.assertRaises(ValueError):
                runner.inventory(value)
        self.assertEqual(runner.inventory("0 tests, 0 benchmarks"), [])

    def test_argument_budget_rejects_a_single_excessive_name(self):
        with self.assertRaises(ValueError):
            runner.batches(["x" * 513], 512)

    def test_owned_cache_reuses_dependencies_and_trims_incremental_first(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            dependency = self.cache.path / "debug" / "deps" / "keep.rlib"
            dependency.parent.mkdir(parents=True)
            dependency.write_bytes(b"verified dependencies")
            incremental = self.cache.path / "debug" / "incremental"
            incremental.mkdir()
            (incremental / "old.o").write_bytes(b"x" * 32768)
            total = runner.footprint(self.cache.path)
            inc_size = runner.footprint(incremental)
            self.cache.trim(total - inc_size + 8192)
            self.assertFalse(incremental.exists())
            self.assertEqual(dependency.read_bytes(), b"verified dependencies")
        with redirect_stdout(io.StringIO()), self.cache.locked():
            self.assertEqual(dependency.read_bytes(), b"verified dependencies")

    def test_larger_cache_cleans_only_owned_debug_not_product_or_external_links(self):
        product = self.repository / "app" / "src-tauri" / "target"
        product.mkdir(parents=True)
        (product / "release").write_bytes(b"keep product")
        with redirect_stdout(io.StringIO()), self.cache.locked():
            debug = self.cache.path / "debug"
            debug.mkdir()
            (debug / "big.o").write_bytes(b"x" * 32768)
            (debug / "outside").symlink_to(product, target_is_directory=True)
            self.cache.trim(8192)
            self.assertFalse(debug.exists())
            self.assertEqual((product / "release").read_bytes(), b"keep product")
            self.assertTrue((self.cache.path / runner.MARKER).exists())

    def test_capacity_pressure_evicts_old_crate_state_before_new_incremental(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            incremental = self.cache.path / "debug" / "incremental"
            old, new = incremental / "old-123", incremental / "new-456"
            old.mkdir(parents=True)
            new.mkdir()
            (old / "old.o").write_bytes(b"x" * 32768)
            (new / "current.o").write_bytes(b"x" * 32768)
            os.utime(old, ns=(1, 1))
            total = runner.footprint(self.cache.path)
            self.cache.trim(total - runner.footprint(old) + 8192)
            self.assertFalse(old.exists())
            self.assertTrue((new / "current.o").exists())

    def test_default_execution_keeps_full_libtest_scheduler(self):
        listed = subprocess.CompletedProcess([], 0, "a::one: test\n1 test, 0 benchmarks\n")
        with patch.object(runner.subprocess, "run", side_effect=[listed, subprocess.CompletedProcess([], 0)]) as run:
            self.assertEqual(runner.run_units(Path("/test"), self.root, {}, 1, 10, None, []), 0)
        self.assertEqual(run.call_args.args[0], ["/test", "--test-threads=10"])

    def test_capacity_pressure_keeps_latest_generation_and_shared_object(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            crate = self.cache.path / "debug/incremental/mon_lib-123"
            old, new = crate / "s-old-version-hash", crate / "s-new-version-hash"
            old.mkdir(parents=True)
            new.mkdir()
            (old / "dep-graph.bin").write_bytes(b"a" * 32768)
            (new / "dep-graph.bin").write_bytes(b"b" * 32768)
            (old / "shared.o").write_bytes(b"x" * 32768)
            os.link(old / "shared.o", new / "shared.o")
            os.utime(old, ns=(1, 1))
            total = runner.footprint(self.cache.path)
            self.cache.trim(total - 32768 + 8192)
            self.assertFalse(old.exists())
            self.assertTrue((new / "dep-graph.bin").exists())
            self.assertEqual((new / "shared.o").read_bytes(), b"x" * 32768)
            with self.assertRaises(ValueError):
                self.cache.remove_debug(True, crate.name, "../outside")
            for entry in ("", ".", "..", "../outside"):
                with self.assertRaises(ValueError):
                    self.cache.remove_debug(True, entry)

    def test_completed_unshared_objects_are_evicted_before_latest_incremental(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            deps = self.cache.path / "debug/deps"
            deps.mkdir(parents=True)
            executable = deps / "mon_lib-0123456789abcdef"
            executable.write_bytes(b"completed test executable")
            executable.chmod(0o700)
            old = deps / (executable.name + ".old.unit.rcgu.o")
            old.write_bytes(b"x" * 32768)
            os.utime(old, ns=(1, 1))
            newest = self.cache.path / "debug/incremental/mon_lib-123/s-current-hash"
            newest.mkdir(parents=True)
            (newest / "dep-graph.bin").write_bytes(b"y" * 32768)
            total = runner.footprint(self.cache.path)
            self.cache.trim(total - runner.footprint(old) + 8192)
            self.assertFalse(old.exists())
            self.assertTrue((newest / "dep-graph.bin").exists())
            self.assertEqual(executable.read_bytes(), b"completed test executable")

    def test_object_cleanup_preserves_shared_unfinished_unmatched_and_external_files(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            deps = self.cache.path / "debug/deps"
            deps.mkdir(parents=True)
            executable = deps / "mon_lib-0123456789abcdef"
            executable.write_bytes(b"executable")
            executable.chmod(0o700)
            os.utime(executable, ns=(10, 10))
            objects = []
            for stem in [executable.name + ".shared.unit", executable.name + ".new.unit",
                         "no_binary-0123456789abcdef.old.unit", "unmatched"]:
                candidate = deps / (stem + ".rcgu.o")
                candidate.write_bytes(b"keep")
                os.utime(candidate, ns=(1, 1))
                objects.append(candidate)
            outside = self.root / "external.o"
            outside.write_bytes(b"outside")
            link = deps / (executable.name + ".link.unit.rcgu.o")
            link.symlink_to(outside)
            shared = self.cache.path / "debug/incremental/shared/current.o"
            shared.parent.mkdir(parents=True)
            os.link(objects[0], shared)
            os.utime(objects[1], ns=(20, 20))
            for artifact, executable_link in [("plain-0123456789abcdef", False),
                                              ("linked-0123456789abcdef", True)]:
                binary = deps / artifact
                if executable_link:
                    binary.symlink_to(executable)
                else:
                    binary.write_bytes(b"not executable")
                candidate = deps / (artifact + ".old.unit.rcgu.o")
                candidate.write_bytes(b"keep")
                os.utime(candidate, ns=(1, 1))
                objects.append(candidate)
            self.cache.remove_completed_objects(1)
            self.assertTrue(all(candidate.exists() for candidate in objects))
            self.assertTrue(link.is_symlink())
            self.assertEqual(outside.read_bytes(), b"outside")
            self.assertEqual(shared.read_bytes(), b"keep")

    def test_object_cleanup_requires_live_owned_cache_lock(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            pass
        with self.assertRaises(ValueError):
            self.cache.remove_completed_objects(1)

    def test_object_cleanup_refuses_linked_deps_directory(self):
        external = self.root / "external"
        external.mkdir()
        keep = external / "keep.o"
        keep.write_bytes(b"outside")
        with redirect_stdout(io.StringIO()), self.cache.locked():
            debug = self.cache.path / "debug"
            debug.mkdir()
            (debug / "deps").symlink_to(external, target_is_directory=True)
            with self.assertRaises(OSError):
                self.cache.remove_completed_objects(1)
        self.assertEqual(keep.read_bytes(), b"outside")

    def test_object_changed_after_inventory_is_not_unlinked(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            deps = self.cache.path / "debug/deps"
            deps.mkdir(parents=True)
            executable = deps / "mon_lib-0123456789abcdef"
            executable.write_bytes(b"completed executable")
            executable.chmod(0o700)
            candidate = deps / (executable.name + ".old.unit.rcgu.o")
            candidate.write_bytes(b"keep")
            os.utime(candidate, ns=(1, 1))
            original_stat = os.stat
            calls = 0

            def stat_with_change(path, *args, **kwargs):
                nonlocal calls
                if path == candidate.name:
                    calls += 1
                    if calls == 2:
                        os.utime(candidate, ns=(2, 2))
                return original_stat(path, *args, **kwargs)

            with patch.object(runner.os, "stat", side_effect=stat_with_change), \
                    self.assertRaisesRegex(ValueError, "changed before cleanup"):
                self.cache.remove_completed_objects(1)
            self.assertEqual(candidate.read_bytes(), b"keep")

    def test_unowned_nonempty_cache_is_preserved(self):
        self.cache.path.mkdir(parents=True)
        keep = self.cache.path / "keep"
        keep.write_text("not ours")
        with redirect_stdout(io.StringIO()), self.assertRaises(ValueError), self.cache.locked():
            pass
        self.assertEqual(keep.read_text(), "not ours")

    def test_foreign_owner_and_replaced_cache_refuse_cleanup(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            marker = self.cache.path / runner.MARKER
            marker.write_text(json.dumps({"schemaVersion": 1, "repository": "/another"}))
            with self.assertRaises(ValueError):
                self.cache.trim(1)
        self.assertTrue(marker.exists())

    def test_symlink_cache_or_incremental_is_not_followed(self):
        external = self.root / "external"
        external.mkdir()
        (external / "keep").write_text("keep")
        self.cache.path.parent.mkdir(parents=True, exist_ok=True)
        self.cache.path.symlink_to(external, target_is_directory=True)
        with redirect_stdout(io.StringIO()), self.assertRaises(ValueError), self.cache.locked():
            pass
        self.cache.path.unlink()
        with redirect_stdout(io.StringIO()), self.cache.locked():
            (self.cache.path / "debug").mkdir()
            (self.cache.path / "debug" / "incremental").symlink_to(external, target_is_directory=True)
            with self.assertRaises(ValueError):
                self.cache.remove_debug(incremental_only=True)
        self.assertEqual((external / "keep").read_text(), "keep")

    def test_broad_root_is_rejected(self):
        for path in [self.repository, self.repository.parent, Path("/"), Path.home()]:
            with self.assertRaises(ValueError):
                runner.Cache(path, self.repository)

    def test_focused_ignored_execution_preserves_filters_exit_and_lock(self):
        with patch.object(runner.subprocess, "run", return_value=subprocess.CompletedProcess([], 101)) as run:
            code = runner.run_units(Path("/test"), self.root, {}, 2, 2, "case", ["--ignored"], 42)
        self.assertEqual(code, 101)
        self.assertEqual(run.call_args.args[0], ["/test", "case", "--ignored", "--test-threads=2"])
        self.assertEqual(run.call_args.kwargs["pass_fds"], (42,))

    def test_normal_execution_dispatches_exact_inventory_without_ignored_override(self):
        listed = subprocess.CompletedProcess([], 0, "a::one: test\nb::two: test\n2 tests, 0 benchmarks\n")
        calls = []
        def run(arguments, **kwargs):
            calls.append(arguments)
            return listed if "--list" in arguments else subprocess.CompletedProcess([], 0)
        with redirect_stdout(io.StringIO()), patch.object(runner.subprocess, "run", side_effect=run):
            self.assertEqual(runner.run_units(Path("/test"), self.root, {}, 2, 2, None, []), 0)
        self.assertEqual(calls[1], ["/test", "a::one", "b::two", "--exact", "--test-threads=2"])
        self.assertNotIn("--ignored", calls[1])

    def test_free_space_floor_refuses_build_and_status_never_builds(self):
        with patch.object(runner, "ROOT", self.repository), redirect_stdout(io.StringIO()):
            with patch.object(runner.shutil, "disk_usage", return_value=shutil_usage(0)), \
                    patch.object(runner, "build") as build:
                with self.assertRaises(ValueError):
                    runner.main(["--cache-dir", str(self.cache.path)])
                build.assert_not_called()
            with patch.object(runner, "build") as build:
                self.assertEqual(runner.main(["--cache-dir", str(self.cache.path), "--status"]), 0)
                build.assert_not_called()

    def test_full_workflow_keeps_all_targets_doc_tests_and_owned_cache(self):
        with patch.object(runner, "ROOT", self.repository), redirect_stdout(io.StringIO()), \
                patch.object(runner, "build", return_value=(0, [Path("/test")])) as build, \
                patch.object(runner, "run_units", return_value=0) as units, \
                patch.object(runner.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as doc:
            self.assertEqual(runner.main(["--cache-dir", str(self.cache.path), "--min-free-mib", "0"]), 0)
        command, environment, cwd, fd = build.call_args.args
        self.assertNotIn("--lib", command)
        self.assertIn("--no-run", command)
        self.assertIn("--locked", command)
        self.assertIn("--offline", command)
        self.assertEqual(command[command.index("--jobs") + 1], str(min(4, os.cpu_count() or 1)))
        self.assertEqual(environment["CARGO_TARGET_DIR"], str(self.cache.path))
        self.assertEqual(units.call_args.args[-1], fd)
        self.assertIn("--doc", doc.call_args.args[0])
        self.assertIn("--jobs", doc.call_args.args[0])
        self.assertEqual(doc.call_args.kwargs["pass_fds"], (fd,))

    def test_build_failure_propagates_without_running_or_retrying_tests(self):
        with patch.object(runner, "ROOT", self.repository), redirect_stdout(io.StringIO()), \
                patch.object(runner, "build", return_value=(13, [])) as build, \
                patch.object(runner, "run_units") as units:
            self.assertEqual(runner.main(["--cache-dir", str(self.cache.path), "--min-free-mib", "0"]), 13)
        self.assertEqual(build.call_count, 1)
        units.assert_not_called()

    def test_build_only_reuses_full_test_profile_without_running_units_or_docs(self):
        with patch.object(runner, "ROOT", self.repository), redirect_stdout(io.StringIO()), \
                patch.object(runner, "build", return_value=(0, [Path("/test")])) as build, \
                patch.object(runner, "run_units") as units, \
                patch.object(runner.subprocess, "run") as doc:
            self.assertEqual(runner.main(["--cache-dir", str(self.cache.path), "--min-free-mib", "0",
                                         "--build-only"]), 0)
        self.assertNotIn("--lib", build.call_args.args[0])
        units.assert_not_called()
        doc.assert_not_called()

    def test_interrupted_workflow_does_not_trim_a_possibly_live_child_cache(self):
        def build(*arguments):
            debug = self.cache.path / "debug"
            debug.mkdir()
            (debug / "still-in-use.o").write_bytes(b"x" * runner.MIB * 2)
            return 0, [Path("/test")]
        with patch.object(runner, "ROOT", self.repository), redirect_stdout(io.StringIO()), \
                patch.object(runner, "build", side_effect=build), \
                patch.object(runner, "run_units", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                runner.main(["--cache-dir", str(self.cache.path), "--min-free-mib", "0",
                             "--max-cache-mib", "1"])
        self.assertTrue((self.cache.path / "debug" / "still-in-use.o").exists())

    def test_inherited_lock_survives_parent_release_until_real_child_exits(self):
        child = None
        try:
            with redirect_stdout(io.StringIO()), self.cache.locked():
                child = subprocess.Popen([sys.executable, "-c", "import os; os.read(0, 1)"],
                                         stdin=subprocess.PIPE, pass_fds=(self.cache.lock_fd,))
            lock_path = self.cache.path.with_name(self.cache.path.name + ".lock")
            probe = """import fcntl, os, sys
fd = os.open(sys.argv[1], os.O_RDWR | os.O_NOFOLLOW)
try:
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
except BlockingIOError:
    sys.exit(23)
finally:
    os.close(fd)
"""
            self.assertEqual(subprocess.run([sys.executable, "-c", probe, str(lock_path)]).returncode, 23)
            child.communicate(input=b"x", timeout=5)
            self.assertEqual(child.returncode, 0)
            self.assertEqual(subprocess.run([sys.executable, "-c", probe, str(lock_path)]).returncode, 0)
        finally:
            if child is not None and child.poll() is None:
                child.kill()
                child.communicate(timeout=5)

    def test_status_never_trims_and_product_target_is_always_rejected(self):
        with redirect_stdout(io.StringIO()), self.cache.locked():
            debug = self.cache.path / "debug"
            debug.mkdir()
            keep = debug / "keep.o"
            keep.write_bytes(b"x" * runner.MIB * 2)
        with patch.object(runner, "ROOT", self.repository), redirect_stdout(io.StringIO()):
            self.assertEqual(runner.main(["--cache-dir", str(self.cache.path), "--status", "--max-cache-mib", "1"]), 0)
        self.assertTrue(keep.exists())
        for path in [self.repository / "app", self.repository / "app/src-tauri/target",
                     self.repository / "app/src-tauri/target/debug/cache"]:
            with self.assertRaises(ValueError):
                runner.Cache(path, self.repository)


def shutil_usage(free):
    from collections import namedtuple
    return namedtuple("usage", "total used free")(free, 0, free)


if __name__ == "__main__":
    unittest.main()
