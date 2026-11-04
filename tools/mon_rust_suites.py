"""Shared Rust suite selection. New tests belong to quick unless listed as heavy.

No Cargo features or Rust profiles change between suites. Ignored libtest children
remain ignored unless explicitly requested by their existing parent tests.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import threading
import time

RULES = Path(__file__).with_name("rust-test-suites.json")
SUITES = ("quick", "heavy", "full")


def heavy_rules(path=RULES):
    value = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(value, dict) or value.get("schemaVersion") != 1 or not isinstance(value.get("heavy"), list):
        raise ValueError("invalid Rust suite definition")
    seen = set()
    for rule in value["heavy"]:
        if not isinstance(rule, dict):
            raise ValueError("invalid heavy test rule")
        keys = set(rule) - {"reason"}
        if keys not in ({"prefix"}, {"name"}) or not isinstance(rule.get("reason"), str) or not rule["reason"].strip():
            raise ValueError("heavy rules require one name/prefix and a reason")
        key = next(iter(keys))
        name = rule[key]
        if (not isinstance(name, str) or not re.fullmatch(r"[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*(?:::)?", name)
                or "::" not in name or (key == "prefix" and not name.endswith("::"))
                or (key == "name" and name.endswith(":")) or name in seen):
            raise ValueError("invalid or duplicate heavy test rule")
        seen.add(name)
    return value["heavy"]


def inventory(output):
    names = [line[:-6] for line in output.splitlines() if line.endswith(": test")]
    summary = re.search(r"^(\d+) tests?, (\d+) benchmarks?$", output, re.MULTILINE)
    if (len(names) != len(set(names)) or len(names) > 25_000 or summary is None
            or int(summary[1]) != len(names) or int(summary[2]) != 0):
        raise ValueError("invalid or missing Rust test inventory")
    return names


def select_tests(names, suite, rules=None):
    if suite not in SUITES:
        raise ValueError("unknown Rust suite")
    rules = heavy_rules() if rules is None else rules
    if len(names) != len(set(names)):
        raise ValueError("duplicate tests in inventory")
    def heavy(name):
        return any(name.startswith(rule["prefix"]) if "prefix" in rule else name == rule["name"]
                   for rule in rules)
    return [name for name in names if suite == "full" or heavy(name) == (suite == "heavy")]


def exact_batches(names, maximum=8000):
    """Bound UTF-16 command length too, for Windows CreateProcess's limit."""
    group, size = [], 0
    for name in names:
        length = len(name.encode("utf-16-le")) // 2 + 3
        if length > maximum:
            raise ValueError("test name exceeds argument budget")
        if group and (size + length > maximum or len(group) == 64):
            yield group
            group, size = [], 0
        group.append(name)
        size += length
    if group:
        yield group


def run_suite(executable, *, cwd, env, suite="full", threads=None, extra=(), pass_fds=(), report_path=None,
              processes=1):
    """Run an explicit partition without changing libtest's ignored-test policy."""
    started = time.monotonic()
    if not 1 <= processes <= 4:
        raise ValueError("test processes must be between 1 and 4")
    options = {"cwd": cwd, "env": env}
    if pass_fds:
        options["pass_fds"] = pass_fds
    base = [str(executable)]
    extra = list(extra)
    def listed(arguments):
        result = subprocess.run(base + arguments + ["--list"], capture_output=True,
                                text=True, encoding="utf-8", timeout=30, check=True, **options)
        return inventory(result.stdout)
    names = listed(extra)
    rules = heavy_rules()
    selected = select_tests(names, suite, rules)
    # Listing/control flags would otherwise turn a test invocation into a silent
    # no-op. Focused full runs retain the legacy argument path in their runner.
    if any(arg in {"--list", "--help", "-h"} for arg in extra):
        raise ValueError("suite execution does not accept list/help arguments")
    if not selected and extra:
        print("[rust-tests] No tests matched the requested suite and filters", flush=True)
        code = 1
    elif not selected and names:
        print(f"[rust-tests] suite={suite}: 0 of {len(names)} tests in this target", flush=True)
        code = 0
    else:
        args = extra[:]
        if threads is not None:
            args.append(f"--test-threads={threads}")
        print(f"[rust-tests] suite={suite}: {len(selected)} of {len(names)} inventoried tests "
              "(ignored tests retain libtest policy)", flush=True)
        if suite == "full" and processes == 1:
            commands = [base + args]
        elif suite == "quick" and "--exact" not in extra and processes == 1:
            skips = [arg for rule in rules for arg in ("--skip", rule.get("prefix", rule.get("name")))]
            # Libtest uses substring matching for --skip. Refuse any mismatch
            # with our anchored classification, including future name collisions.
            if set(listed(extra + skips)) != set(selected):
                raise ValueError("libtest skip selection differs from the declared quick suite")
            commands = [base + args + skips]
        else:
            # --exact applies to all filters, including --skip. Materialize the
            # inventory's selection and leave only execution options on argv.
            allowed = {"--ignored", "--include-ignored", "--nocapture", "--show-output", "--quiet", "-q"}
            execution = [arg for arg in args if arg in allowed or arg.startswith("--test-threads=")]
            # Spread long modules across workers instead of queuing an entire
            # native-history/SQLite module behind one process's batch barrier.
            modules = {}
            for name in selected:
                modules.setdefault(name.rsplit("::", 1)[0], []).append(name)
            ordered = []
            while modules:
                for module in list(modules):
                    ordered.append(modules[module].pop(0))
                    if not modules[module]:
                        del modules[module]
            commands = [base + group + ["--exact"] + execution for group in exact_batches(ordered)]
        code = 0
        failed = threading.Event()
        def execute(command):
            if failed.is_set():
                return None
            result = subprocess.run(command, check=False, **options).returncode
            if result:
                failed.set()
            return result
        if processes == 1:
            for command in commands:
                code = execute(command)
                if code:
                    break
        else:
            print(f"[rust-tests] {len(commands)} exact batches; {processes} processes x {threads} threads", flush=True)
            with ThreadPoolExecutor(max_workers=processes) as pool:
                codes = list(pool.map(execute, commands))
            code = next((value for value in codes if value), 0)
    if report_path:
        write_report(report_path, {"suite": suite, "executable": str(executable),
                                  "selected": selected, "inventoried": len(names),
                                  "seconds": time.monotonic() - started, "exitCode": code})
    return code


def write_report(path, values):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    host = platform.uname()
    path.write_text(json.dumps({"schemaVersion": 1, "platform": f"{host.system} {host.release} {host.machine}",
                                "cpuCount": os.cpu_count(), **values}, indent=2) + "\n", encoding="utf-8")


def profile_tests(executable, *, cwd, suite, report_path, tests=(), repeat=1, env=None):
    """Measure runnable exact tests, including process startup, without reclassification."""
    if not 1 <= repeat <= 20:
        raise ValueError("diagnostic repeat count must be between 1 and 20")
    options = {"cwd": cwd}
    if env is not None:
        options["env"] = env
    output = subprocess.run([str(executable), "--list"],
                            capture_output=True, text=True, check=True, **options)
    names = select_tests(inventory(output.stdout), suite)
    # --list includes ignored children. Obtain only runnable tests explicitly.
    ignored_output = subprocess.run([str(executable), "--list", "--ignored"],
                                    capture_output=True, text=True, check=True, **options)
    ignored = set(inventory(ignored_output.stdout))
    runnable = [name for name in names if name not in ignored]
    selected = list(dict.fromkeys(tests)) if tests else runnable
    if not selected:
        raise ValueError("no runnable tests in the requested diagnostic suite")
    for name in selected:
        if name not in runnable:
            raise ValueError(f"diagnostic test is unknown, ignored or outside the {suite} suite: {name}")
    results = []
    code = 0
    try:
        for iteration in range(1, repeat + 1):
            for name in selected:
                started = time.monotonic()
                result = subprocess.run([str(executable), name, "--exact", "--test-threads=1"], **options)
                elapsed = time.monotonic() - started
                results.append({"name": name, "iteration": iteration,
                                "seconds": elapsed, "exitCode": result.returncode})
                print(f"[rust-tests] diagnostic {iteration}/{repeat}: {elapsed:.3f}s {name}", flush=True)
                if result.returncode:
                    code = result.returncode
                    break
            if code:
                break
    except (OSError, subprocess.SubprocessError):
        code = 1
        raise
    finally:
        write_report(report_path, {"suite": suite, "diagnosticOnly": True,
                                  "selected": selected, "repeat": repeat,
                                  "tests": sorted(results, key=lambda item: -item["seconds"]),
                                  "exitCode": code})
    return code


def main(argv=None):
    parser = argparse.ArgumentParser(description="Profile tests individually; diagnostic timings are not suite wall time")
    parser.add_argument("executable", type=Path)
    parser.add_argument("--suite", choices=SUITES, default="full")
    parser.add_argument("--cwd", type=Path, default=Path(__file__).resolve().parents[1] / "app/src-tauri")
    parser.add_argument("--profile-json", type=Path, required=True)
    parser.add_argument("--test", action="append", default=[], metavar="EXACT_NAME",
                        help="profile only this exact runnable test in the selected suite; repeat for multiple tests")
    parser.add_argument("--repeat", type=int, choices=range(1, 21), default=1,
                        help="number of diagnostic iterations; default 1")
    args = parser.parse_args(argv)
    try:
        return profile_tests(args.executable.resolve(strict=True), cwd=args.cwd, suite=args.suite,
                             report_path=args.profile_json, tests=args.test, repeat=args.repeat)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Rust diagnostic failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
