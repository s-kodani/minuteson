#!/usr/bin/env python3
"""Build the native library tests once and run the resulting test executable."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
TEST_TARGET = "mon_desktop_lib"
SWIFT_FIXTURE = "swift_generated_pc_only_capture"


def build_library_tests(manifest):
    command = ["cargo", "test", "--manifest-path", str(manifest), "--locked",
               "--lib", "--no-run", "--message-format=json"]
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
                    and isinstance(target, dict) and target.get("name") == TEST_TARGET
                    and isinstance(profile, dict) and profile.get("test") is True
                    and isinstance(message.get("executable"), str)
                    and message["executable"]):
                executables.append(Path(message["executable"]))
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


def run_native_tests(root=None, swift_fixture=False):
    manifest = (ROOT if root is None else Path(root)).resolve() / "app/src-tauri/Cargo.toml"
    code, executables, linked_paths = build_library_tests(manifest)
    if code:
        return code
    if len(executables) != 1:
        raise ValueError(f"expected one {TEST_TARGET} test executable, found {len(executables)}")
    executable = executables[0]
    if not executable.is_absolute() or not executable.is_file():
        raise ValueError(f"Cargo did not produce an existing absolute test executable: {executable}")
    # Cargo normally launches a package's tests from the package directory.
    # Reuse that boundary and Cargo's runtime search paths for both invocations.
    environment = runtime_environment(executable, linked_paths, manifest.parent)
    command = [str(executable)]
    code = subprocess.run(command, cwd=manifest.parent, env=environment, check=False).returncode
    if code or not swift_fixture:
        return code
    return subprocess.run(command + [SWIFT_FIXTURE, "--ignored"],
                          cwd=manifest.parent, env=environment, check=False).returncode


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--swift-fixture", action="store_true",
                        help="also run the ignored Swift capture fixture with the same executable")
    args = parser.parse_args(argv)
    try:
        return run_native_tests(swift_fixture=args.swift_fixture)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Native CI tests failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
