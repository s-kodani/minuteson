#!/usr/bin/env python3
"""Build native tests once; select quick, heavy or full without profile changes."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
TEST_TARGET = "mon_desktop_lib"
SWIFT_FIXTURE = "swift_generated_pc_only_capture"


def build_library_tests(manifest, *, all_targets=False, targets=None):
    command = ["cargo", "test", "--manifest-path", str(manifest), "--locked",
               *([] if all_targets else ["--lib"]), "--no-run", "--message-format=json"]
    executables = []
    linked_paths = []
    with subprocess.Popen(command, cwd=manifest.parent, stdout=subprocess.PIPE,
                          text=True, encoding="utf-8", errors="replace") as child:
        for line in child.stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                print(line, end="", flush=True)
                continue
            if not isinstance(message, dict):
                continue
            if message.get("reason") == "build-script-executed":
                linked_paths.extend(path for path in message.get("linked_paths", [])
                                    if isinstance(path, str))
            if message.get("reason") == "compiler-message":
                diagnostic = message.get("message", {})
                rendered = (diagnostic.get("rendered") or diagnostic.get("message")) \
                    if isinstance(diagnostic, dict) else None
                if isinstance(rendered, str):
                    print(rendered, end="" if rendered.endswith("\n") else "\n",
                          file=sys.stderr, flush=True)
            target, profile = message.get("target"), message.get("profile")
            if (message.get("reason") == "compiler-artifact"
                    and isinstance(target, dict)
                    and (all_targets or target.get("name") == TEST_TARGET)
                    and isinstance(profile, dict) and profile.get("test") is True
                    and isinstance(message.get("executable"), str)
                    and message["executable"]):
                executable = Path(message["executable"])
                executables.append(executable)
                if targets is not None:
                    targets[executable] = target.get("name")
        return child.wait(), executables, linked_paths


def runtime_environment(executable, linked_paths, cwd):
    """Keep Cargo's runtime library search boundary when launching directly."""
    environment = dict(os.environ)
    query = subprocess.run([environment.get("RUSTC", "rustc"), "--print", "target-libdir"],
                           cwd=cwd, env=dict(environment), stdout=subprocess.PIPE, check=True,
                           text=True, encoding="utf-8", errors="replace")
    lines = query.stdout.strip().splitlines()
    if len(lines) != 1 or not Path(lines[0]).is_absolute() or not Path(lines[0]).is_dir():
        raise ValueError("rustc did not report an existing absolute target library directory")
    deps = executable.parent
    output = deps.parent
    target = output.parent.resolve()
    paths = []
    for linked in linked_paths:
        kind, separator, value = linked.partition("=")
        path = Path(value if separator and kind in {"dependency", "crate", "native", "framework", "all"}
                    else linked)
        path = (path if path.is_absolute() else cwd / path).resolve()
        if path.is_relative_to(target):
            paths.append(str(path))
    paths.extend((str(deps), str(output), str(Path(lines[0]).resolve())))
    paths = list(dict.fromkeys(paths))
    variable = {"win32": "PATH", "darwin": "DYLD_FALLBACK_LIBRARY_PATH"}.get(
        sys.platform, "LD_LIBRARY_PATH")
    separator = ";" if sys.platform == "win32" else ":"
    if variable in environment:
        paths.append(environment[variable])
    elif sys.platform == "darwin":
        home = Path(environment.get("HOME") or Path.home())
        paths.extend((str(home / "lib"), "/usr/local/lib", "/usr/lib"))
    environment[variable] = separator.join(paths)
    return environment


def _run_native_tests(root, swift_fixture, suite, all_targets, threads, processes, metrics):
    manifest = (ROOT if root is None else Path(root)).resolve() / "app/src-tauri/Cargo.toml"
    targets = {}
    started = time.monotonic()
    try:
        if all_targets:
            code, executables, linked_paths = build_library_tests(
                manifest, all_targets=True, targets=targets)
        else:
            code, executables, linked_paths = build_library_tests(manifest)
    finally:
        metrics["buildSeconds"] = time.monotonic() - started
    if code:
        return code
    libraries = ([path for path in executables if targets[path] == TEST_TARGET]
                 if all_targets else executables)
    if len(libraries) != 1:
        raise ValueError(f"expected one {TEST_TARGET} test executable, found {len(executables)}")
    if len(executables) != len(set(executables)):
        raise ValueError("Cargo produced duplicate test executables")
    for executable in executables:
        if not executable.is_absolute() or not executable.is_file():
            raise ValueError(f"Cargo did not produce an existing absolute test executable: {executable}")
    # Cargo normally launches a package's tests from the package directory.
    # Reuse that boundary and Cargo's runtime search paths for both invocations.
    started = time.monotonic()
    try:
        environments = {}
        for executable in executables:
            is_library = executable == libraries[0]
            if suite == "heavy" and not is_library:
                continue
            environment = runtime_environment(executable, linked_paths, manifest.parent)
            environments[executable] = environment
            if suite != "full" and is_library:
                # Import lazily so the existing full-library entry point stays
                # independent of suite inventory and diagnostic subprocesses.
                try:
                    from tools.mon_rust_suites import run_suite
                except ModuleNotFoundError:
                    from mon_rust_suites import run_suite
                parallel = {"processes": processes} if processes != 1 else {}
                code = run_suite(executable, cwd=manifest.parent, env=environment,
                                 suite=suite, threads=threads, **parallel)
            else:
                command = [str(executable)]
                if threads is not None:
                    command.append(f"--test-threads={threads}")
                code = subprocess.run(command, cwd=manifest.parent, env=environment,
                                      check=False).returncode
            if code:
                return code
        if swift_fixture:
            executable = libraries[0]
            environment = environments[executable]
            command = [str(executable), SWIFT_FIXTURE, "--ignored"]
            if threads is not None:
                command.append(f"--test-threads={threads}")
            code = subprocess.run(command, cwd=manifest.parent, env=environment,
                                  check=False).returncode
            if code:
                return code
        if all_targets and suite != "heavy":
            return subprocess.run(["cargo", "test", "--manifest-path", str(manifest),
                                   "--locked", "--doc"], cwd=manifest.parent,
                                  check=False).returncode
        return 0
    finally:
        metrics["executionSeconds"] = time.monotonic() - started


def run_native_tests(root=None, swift_fixture=False, *, suite="full", all_targets=False,
                     threads=None, processes=1, timings_json=None):
    if suite not in ("quick", "heavy", "full"):
        raise ValueError("unknown Rust test suite")
    if swift_fixture and suite == "quick":
        raise ValueError("the Swift integration fixture requires the heavy or full suite")
    if threads is not None and not 1 <= threads <= 64:
        raise ValueError("test threads must be between 1 and 64")
    if not 1 <= processes <= 4:
        raise ValueError("test processes must be between 1 and 4")
    if suite == "full" and processes != 1:
        raise ValueError("parallel processes require the quick or heavy suite")
    host = platform.uname()
    metrics = {"schemaVersion": 1, "suite": suite,
               "platform": f"{host.system}-{host.release}-{host.machine}",
               "cpuCount": os.cpu_count(), "allTargets": all_targets, "threads": threads,
               "processes": processes,
               "buildSeconds": 0,
               "executionSeconds": 0, "exitCode": 1}
    started = time.monotonic()
    try:
        metrics["exitCode"] = _run_native_tests(
            root, swift_fixture, suite, all_targets, threads, processes, metrics)
        return metrics["exitCode"]
    finally:
        metrics["totalSeconds"] = time.monotonic() - started
        print(f"[rust-tests] Suite {suite}; build {metrics['buildSeconds']:.2f}s; "
              f"execution {metrics['executionSeconds']:.2f}s; "
              f"total {metrics['totalSeconds']:.2f}s; exit {metrics['exitCode']}", flush=True)
        if timings_json is not None:
            path = Path(timings_json)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(metrics, indent=2) + "\n", encoding="utf-8")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--swift-fixture", action="store_true",
                        help="also run the ignored Swift capture fixture with the same executable")
    parser.add_argument("--suite", choices=("quick", "heavy", "full"), default="full")
    parser.add_argument("--all-targets", action="store_true",
                        help="also build and run package binary/integration targets and doc tests")
    parser.add_argument("--threads", type=int, choices=range(1, 65))
    parser.add_argument("--processes", type=int, choices=range(1, 5), default=1,
                        help="opt-in exact batching for quick/heavy; defaults to one scheduler")
    parser.add_argument("--timings-json", type=Path)
    args = parser.parse_args(argv)
    try:
        return run_native_tests(swift_fixture=args.swift_fixture, suite=args.suite,
                                all_targets=args.all_targets, threads=args.threads,
                                processes=args.processes,
                                timings_json=args.timings_json)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Native CI tests failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
