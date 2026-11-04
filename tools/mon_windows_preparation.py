"""Reuse verified unsigned Windows preparation within one Actions job only."""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
import uuid

import mon_windows_build_evidence as evidence

ROOT = Path(__file__).resolve().parents[1]
OUTPUT_TREES = ("app/dist", ".cache/windows-runtime/package", ".cache/windows-third-party-notices")
OUTPUT_FILES = (
    "probes/windows-capture/build/Release/mon-windows-capture.exe",
    "app/src-tauri/binaries/mon-windows-capture-x86_64-pc-windows-msvc.exe",
    ".cache/windows-capture-headers/LICENSE.MIT",
)
REQUIRED = (
    "app/dist/index.html",
    ".cache/windows-third-party-notices/THIRD_PARTY_NOTICES.txt",
    ".cache/windows-third-party-notices/third-party-inventory.json",
    *OUTPUT_FILES,
    *(".cache/windows-runtime/package/" + name for name in (
        "catalog.json", "runtime-index.json", "runtime.tar", "embedding-index.json",
        "mon-speaker-embedding.exe", "workers/runtime-index.json", "workers/mon-whisper-worker.exe",
        "workers/mon-llama-worker.exe", "workers/mon-sherpa-worker.exe", "workers/mon-transducer-worker.exe",
        "workers/mon-whisper-vulkan-worker.exe", "workers/mon-llama-vulkan-worker.exe", "asr/bin/mon-transducer-worker.exe",
    )),
)


def regular(root, path):
    path = evidence.workspace_path(root, path)
    if not path.is_file() or path.is_symlink() or (hasattr(path, "is_junction") and path.is_junction()):
        raise ValueError("Preparation requires ordinary files inside the workspace")
    return path


def session(root):
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise ValueError("Preparation receipts can only be reused within an Actions job")
    values = {key: os.environ.get(key, "") for key in ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "GITHUB_JOB", "RUNNER_TEMP")}
    if not all(values.values()):
        raise ValueError("Actions run, attempt, job and runner temporary directory are required")
    temporary = Path(values["RUNNER_TEMP"])
    if not temporary.is_absolute() or not temporary.is_dir() or temporary.resolve() != temporary:
        raise ValueError("Runner temporary directory must be an ordinary absolute directory")
    return {**values, "workspace": str(root)}


def receipt_path(root, path):
    temporary = Path(session(root)["RUNNER_TEMP"])
    path = Path(path)
    if not path.is_absolute():
        raise ValueError("Preparation receipt requires an absolute path")
    if path == temporary or not path.is_relative_to(temporary) or path.resolve() != path:
        raise ValueError("Preparation receipt must stay inside this runner's temporary directory")
    return path


def command(*arguments):
    return subprocess.run(arguments, check=True, capture_output=True, text=True,
                          encoding="utf8", errors="replace").stdout.strip()


def toolchain():
    return {"platform": sys.platform, "architecture": platform.machine(), "image": os.environ.get("ImageVersion", ""),
            "python": platform.python_version(), "rust": command("rustc", "-vV"),
            "cmake": command("cmake", "--version"), "node": command("node", "--version"),
            "flags": {key: os.environ.get(key, "") for key in (
                "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CFLAGS", "CXXFLAGS", "VCToolsVersion", "WindowsSDKVersion")}}


def identity(root):
    if os.environ.get("MON_WINDOWS_RELEASE_SIGNING", "0") != "0":
        raise ValueError("Unsigned preparation cannot be reused for a signed build")
    return {"session": session(root), "source": evidence.source_identity(root), "toolchain": toolchain(), "signing": "unsigned"}


def inventory(root):
    for name in REQUIRED:
        regular(root, root / name)
    paths = {regular(root, root / name) for name in OUTPUT_FILES}
    for name in OUTPUT_TREES:
        tree = evidence.workspace_path(root, root / name)
        if not tree.is_dir():
            raise ValueError("Prepared output directory is missing")
        for path in tree.rglob("*"):
            evidence.workspace_path(root, path)
            if path.is_symlink() or (hasattr(path, "is_junction") and path.is_junction()):
                raise ValueError("Prepared outputs cannot contain links")
            if path.is_file():
                paths.add(regular(root, path))
    return [{"path": path.relative_to(root).as_posix(), "bytes": path.stat().st_size,
             "sha256": evidence.sha256_file(path)} for path in sorted(paths)]


def capture_configuration(root):
    build = root / "probes/windows-capture/build"
    files = [regular(root, build / "CMakeCache.txt")]
    compilers = sorted((build / "CMakeFiles").glob("*/CMakeCXXCompiler.cmake"))
    if not compilers:
        raise ValueError("Prepared capture compiler identity is missing")
    files.extend(regular(root, path) for path in compilers)
    executables = {}
    for path in compilers:
        match = re.search(r'^set\(CMAKE_CXX_COMPILER "([^"]+)"\)', path.read_text(encoding="utf8"), re.MULTILINE)
        compiler = Path(match[1]) if match else None
        if compiler is None or not compiler.is_absolute() or not compiler.is_file() or compiler.is_symlink():
            raise ValueError("Prepared capture compiler executable is missing")
        executables[str(compiler)] = evidence.sha256_file(compiler)
    return {"configurationFiles": {path.relative_to(root).as_posix(): evidence.sha256_file(path) for path in files},
            "compilerExecutables": executables}


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_name(path.name + "." + uuid.uuid4().hex + ".pending")
    try:
        pending.write_text(json.dumps(value, indent=2) + "\n", encoding="utf8")
        os.replace(pending, path)
    finally:
        if pending.exists():
            pending.unlink()


def begin(root, path):
    path = receipt_path(root, path)
    if path.exists():
        raise ValueError("A new preparation receipt path is required")
    write_json(path, {"schemaVersion": 1, "status": "preparing", "identity": identity(root)})


def read_receipt(root, path, status):
    path = receipt_path(root, path)
    value = json.loads(path.read_text(encoding="utf8"))
    if not isinstance(value, dict) or value.get("schemaVersion") != 1 or value.get("status") != status or value.get("identity") != identity(root):
        raise ValueError("Preparation receipt does not match this unsigned job, source or toolchain")
    return path, value


def record(root, path):
    path, value = read_receipt(root, path, "preparing")
    outputs = inventory(root)
    configuration = capture_configuration(root)
    # Detect source/toolchain changes during output hashing as well.
    if value["identity"] != identity(root):
        raise ValueError("Preparation inputs changed while recording outputs")
    write_json(path, {**value, "status": "ready", "captureConfiguration": configuration, "outputs": outputs})


def verify(root, path):
    _, value = read_receipt(root, path, "ready")
    if value.get("captureConfiguration") != capture_configuration(root) or value.get("outputs") != inventory(root):
        raise ValueError("Prepared Windows outputs changed or are incomplete")
    if value["identity"] != identity(root):
        raise ValueError("Preparation inputs changed while verifying outputs")


def run_timed(report, phase, arguments):
    started, code, failure = time.monotonic(), 1, None
    value = json.loads(report.read_text(encoding="utf8")) if report.exists() else {"schemaVersion": 1, "phases": []}
    row = {"phase": phase, "seconds": 0, "exitCode": None, "status": "running"}
    value["phases"].append(row)
    write_json(report, value)
    try:
        code = subprocess.run(arguments, check=False).returncode
        return code
    except BaseException as error:
        failure = type(error).__name__
        raise
    finally:
        row.update({"seconds": time.monotonic() - started, "exitCode": code,
                    "status": "success" if code == 0 else "failure",
                    **({"failureType": failure} if failure else {})})
        write_json(report, value)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    for action in ("begin", "record", "verify"):
        sub = commands.add_parser(action)
        sub.add_argument("--receipt", type=Path, required=True)
    timed = commands.add_parser("run")
    timed.add_argument("--report", type=Path, required=True)
    timed.add_argument("--phase", required=True)
    timed.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    if args.action == "run":
        arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
        if not arguments:
            parser.error("a command is required after --")
        return run_timed(args.report, args.phase, arguments)
    globals()[args.action](ROOT.resolve(), args.receipt)
    print("Windows preparation " + args.action + ": verified unsigned same-job resources")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        sys.exit("Windows preparation failed: " + str(error))
