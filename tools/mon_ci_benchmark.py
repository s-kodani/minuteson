#!/usr/bin/env python3
"""Disposable hosted CI measurements; never changes repository build defaults."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
import tomllib
import uuid


class BenchmarkError(ValueError):
    pass


def query(command, *, cwd=None, timeout=15):
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True,
                            encoding="utf-8", errors="replace", timeout=timeout, check=False)
    if result.returncode:
        raise subprocess.CalledProcessError(result.returncode, command)
    return result.stdout.strip()


def verify_source(root, expected):
    if not root.is_absolute() or not root.is_dir() or root.resolve() != root:
        raise BenchmarkError("--root must be an existing canonical absolute checkout directory")
    if not re.fullmatch(r"[0-9a-f]{40}", expected):
        raise BenchmarkError("--source-sha must be the exact 40-character lowercase commit SHA")
    actual = query(["git", "-C", str(root), "rev-parse", "HEAD"], timeout=10)
    if actual != expected:
        raise BenchmarkError("checked-out HEAD does not match --source-sha")
    if query(["git", "-C", str(root), "status", "--porcelain", "--untracked-files=no"], timeout=10):
        raise BenchmarkError("benchmark requires unchanged tracked source files")
    return actual


def require_disposable_workspace(root):
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise BenchmarkError("cache measurements may delete only a disposable GitHub Actions workspace")
    workspace = os.environ.get("GITHUB_WORKSPACE")
    temporary = os.environ.get("RUNNER_TEMP")
    if not workspace or Path(workspace).resolve() != root:
        raise BenchmarkError("--root must be this job's GITHUB_WORKSPACE")
    if not temporary or not Path(temporary).is_absolute() or not Path(temporary).is_dir():
        raise BenchmarkError("cache measurements require an existing absolute RUNNER_TEMP")
    if (root == Path(temporary).resolve() or root in Path(temporary).resolve().parents
            or Path(temporary).resolve() in root.parents):
        raise BenchmarkError("RUNNER_TEMP must be outside the source workspace")


def report_path(root, requested):
    if not root.is_absolute():
        raise BenchmarkError("--root must be absolute")
    path = requested if requested.is_absolute() else root / requested
    path = Path(os.path.abspath(path))
    if path.resolve() != path or path.suffix != ".json":
        raise BenchmarkError("report must be an ordinary .json path without linked parents")
    if path.is_relative_to(root):
        if not path.is_relative_to(root / "output/rust-tests"):
            raise BenchmarkError("reports inside the checkout must stay under output/rust-tests")
        tracked = query(["git", "-C", str(root), "ls-files", "--", path.relative_to(root).as_posix()], timeout=10)
        if tracked:
            raise BenchmarkError("cannot replace a tracked source file with a report")
    else:
        temporary = os.environ.get("RUNNER_TEMP")
        if not temporary or not path.is_relative_to(Path(temporary).resolve()):
            raise BenchmarkError("reports outside the checkout must stay under RUNNER_TEMP")
    return path


def checkpoint_report(path, report):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_name(path.name + "." + uuid.uuid4().hex + ".pending")
    try:
        pending.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        os.replace(pending, path)
    finally:
        pending.unlink(missing_ok=True)


def context(root):
    keys = ("RUNNER_OS", "RUNNER_ARCH", "RUNNER_ENVIRONMENT", "ImageOS", "ImageVersion",
            "MON_CI_RUST_CACHE_HIT", "CARGO_INCREMENTAL", "CARGO_PROFILE_TEST_INCREMENTAL",
            "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_DIR", "DEVELOPER_DIR")
    result = {"platform": platform.platform(), "cpuCount": os.cpu_count(),
              "pythonVersion": platform.python_version(),
              "environment": {key: os.environ[key] for key in keys if key in os.environ},
              "toolchain": {}}
    commands = {"rustc": [os.environ.get("RUSTC", "rustc"), "-vV"],
                "cargo": ["cargo", "--version"], "cmake": ["cmake", "--version"],
                "node": ["node", "--version"]}
    if sys.platform == "darwin":
        commands.update(swift=["xcrun", "swift", "--version"], sdk=["xcrun", "--show-sdk-version"])
    for name, command in commands.items():
        try:
            result["toolchain"][name] = query(command, cwd=root, timeout=15)
        except (OSError, subprocess.SubprocessError):
            result["toolchain"][name] = None
    return result


def run_command(command, *, cwd, env=None, timeout=None):
    """Keep tool/build output in the job log; reports contain no process env dump."""
    print("[benchmark] " + json.dumps([str(value) for value in command]), flush=True)
    started = time.monotonic()
    result = subprocess.run(command, cwd=cwd, env=env, timeout=timeout, check=False)
    if result.returncode:
        raise subprocess.CalledProcessError(result.returncode, command)
    return time.monotonic() - started


def rust_benchmark(root, report, checkpoint, run_command):
    """Audit the proposed override against the environment used by actual CI."""
    source = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    profile = source.get("profile", {}).get("test", {})
    package = profile.get("package", {}).get("mon-desktop", {})
    audit = {"kind": "effective-settings-audit", "candidate": "profile.test.package.mon-desktop.incremental=false",
             "sourceProfileIncremental": profile.get("incremental"),
             "sourcePackageIncremental": package.get("incremental"),
             "cargoIncrementalEnvironment": os.environ.get("CARGO_INCREMENTAL"),
             "eligible": False, "trials": [], "status": "inconclusive"}
    report["rust"] = audit
    checkpoint()
    if os.environ.get("CARGO_INCREMENTAL") != "0":
        raise BenchmarkError("effective-settings audit requires actual inherited CARGO_INCREMENTAL=0 from the pinned rust-cache action; do not clear or synthesize it")
    if profile.get("incremental") is not True:
        raise BenchmarkError("frozen source no longer declares the expected profile.test.incremental=true")
    audit.update(status="redundant", effectiveCurrentIncremental=False, effectiveCandidateIncremental=False,
                 reason="The pinned rust-cache action already exports CARGO_INCREMENTAL=0, overriding both the source default and proposed package setting. No build, clean or A/B trial is warranted.")
    checkpoint()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--mode", choices=("cache", "rust"), required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args(argv)
    # Validate the destination before writing even an error report.
    try:
        destination = report_path(args.root, args.report)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Benchmark cannot write its report: {error}", file=sys.stderr)
        return 1
    report = {"schemaVersion": 1, "mode": args.mode, "sourceSha": None,
              "expectedSourceSha": args.source_sha, "workflowSha": os.environ.get("GITHUB_SHA"),
              "harnessSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "startedAt": datetime.now(timezone.utc).isoformat(), "completed": False, "exitCode": 1}
    started = time.monotonic()
    source_verified = False
    checkpoint = lambda: checkpoint_report(destination, report)
    try:
        checkpoint()
        report["sourceSha"] = verify_source(args.root, args.source_sha)
        source_verified = True
        sys.path.insert(0, str(args.root / "tools"))
        report["context"] = context(args.root)
        checkpoint()
        if args.mode == "cache":
            require_disposable_workspace(args.root)
            cache_benchmark(args.root, report, checkpoint, run_command)
        else:
            rust_benchmark(args.root, report, checkpoint, run_command)
        report.update(completed=True, exitCode=0)
    except KeyboardInterrupt:
        report.update(error="benchmark interrupted", exitCode=130)
    except Exception as error:
        report.update(error=str(error), exitCode=error.returncode if isinstance(error, subprocess.CalledProcessError) else 1)
        print(f"Benchmark failed: {error}", file=sys.stderr)
    finally:
        if source_verified:
            try:
                verify_source(args.root, args.source_sha)
                report["sourceUnchanged"] = True
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                report.update(sourceUnchanged=False, completed=False, exitCode=1)
                report.setdefault("error", str(error))
        report["totalSeconds"] = time.monotonic() - started
        checkpoint()
    return report["exitCode"]


# Native cache experiments use only frozen tool-owned paths. These helpers are
# concatenated into the standalone manual benchmark script, never repo tools.
def cache_scopes(component):
    return {
        'windows-runtime': ('.cache/windows-runtime/whisper-source', '.cache/windows-runtime/whisper-build',
                            '.cache/windows-runtime/workers-source-*', '.cache/windows-runtime/workers-build-*',
                            '.cache/vkw-*', '.cache/vkl-*'),
        'windows-capture': ('probes/windows-capture/build',),
        'macos-capture': ('probes/macos-capture/.build',),
        'macos-workers': ('.cache/inference-runtime/intermediates', '.cache/inference-runtime/downloads'),
    }[component]


def cache_guard_path(root, path):
    import pathlib
    root, path = pathlib.Path(root), pathlib.Path(path)
    if root.resolve() != root.absolute() or not root.is_dir():
        raise ValueError('Cache benchmark needs an ordinary canonical workspace')
    if not path.is_absolute() or path == root or not path.is_relative_to(root):
        raise ValueError('Cache operation escaped the disposable workspace')
    # Check ancestors only: a literal link at the selected path is unlinked,
    # never followed. Swift .build contains legitimate descendant links.
    for parent in path.parents:
        if parent == root:
            break
        if parent.is_symlink() or (hasattr(parent, 'is_junction') and parent.is_junction()):
            raise ValueError('Cache operation cannot traverse a linked parent')
    return path


def cache_member_allowed(name, scopes):
    import pathlib
    path = pathlib.PurePosixPath(name)
    if path.is_absolute() or not path.parts or any(p in ('..', '.') for p in path.parts) or '\\' in name or ':' in name:
        return False
    return any(len(path.parts) >= len(pathlib.PurePosixPath(scope).parts)
               and pathlib.PurePosixPath(*path.parts[:len(pathlib.PurePosixPath(scope).parts)]).match(scope)
               for scope in scopes)


def cache_component_roots(root, component, definition):
    import pathlib
    scopes = cache_scopes(component)
    if tuple(definition.build_paths(component)) != scopes:
        raise ValueError('Frozen native cache scopes do not match benchmark allowlist')
    paths = set()
    for scope in scopes:
        candidates = root.glob(scope) if '*' in scope else (root / scope,)
        for path in candidates:
            path = cache_guard_path(root, pathlib.Path(path))
            if not cache_member_allowed(path.relative_to(root).as_posix(), scopes):
                raise ValueError('Unexpected native cache path')
            if path.exists() or path.is_symlink():
                paths.add(path)
    return sorted(paths)


def cache_remove_path(root, path, allowed):
    import shutil
    path = cache_guard_path(root, path)
    if not cache_member_allowed(path.relative_to(root).as_posix(), allowed):
        raise ValueError('Cache cleanup path is outside its owned allowlist')
    if path.is_symlink() or (hasattr(path, 'is_junction') and path.is_junction()):
        path.unlink() if path.is_symlink() else path.rmdir()
    elif path.is_dir():
        shutil.rmtree(path)
    elif path.exists():
        path.unlink()


def cache_remove_components(root, components, definition):
    for component in components:
        for path in cache_component_roots(root, component, definition):
            cache_remove_path(root, path, cache_scopes(component))


def cache_clean_generated(root, host):
    import re
    if host == 'win32':
        import mon_windows_preparation as preparation
        import mon_windows_vulkan_sdk as sdk
        allowed = ('app/dist', '.cache/windows-runtime/package', '.cache/windows-third-party-notices')
        if tuple(preparation.OUTPUT_TREES) != allowed:
            raise ValueError('Unexpected frozen Windows generated output scope')
        helper = 'app/src-tauri/binaries/mon-windows-capture-x86_64-pc-windows-msvc.exe'
        if helper not in preparation.OUTPUT_FILES or not re.fullmatch(r'\d+(?:\.\d+){2,3}', sdk.VERSION):
            raise ValueError('Unexpected frozen Windows helper/SDK path')
        paths = (*allowed, helper, '.cache/windows-tools/VulkanSDK-' + sdk.VERSION)
    else:
        paths = ('.cache/inference-runtime/package',)
    for name in paths:
        cache_remove_path(root, root / name, paths)
    return list(paths)


def cache_validate_member(member, scopes):
    import pathlib
    import posixpath
    if not cache_member_allowed(member.name, scopes) or not (member.isdir() or member.isfile() or member.issym() or member.islnk()):
        raise ValueError('Native cache archive contains an unexpected member')
    if member.issym() or member.islnk():
        target = pathlib.PurePosixPath(member.linkname)
        if target.is_absolute() or '\\' in member.linkname or ':' in member.linkname:
            raise ValueError('Native cache archive contains an unsafe link')
        joined = (posixpath.join(posixpath.dirname(member.name), member.linkname)
                  if member.issym() else member.linkname)
        if not cache_member_allowed(posixpath.normpath(joined), scopes):
            raise ValueError('Native cache link leaves its allowed component')


def cache_snapshot(root, component, definition, archive):
    import tarfile
    import time
    started = time.monotonic()
    paths = cache_component_roots(root, component, definition)
    if not paths:
        raise ValueError('Native preparation produced no intermediates for ' + component)
    with tarfile.open(archive, 'w', dereference=False) as output:
        for path in paths:
            output.add(path, arcname=path.relative_to(root).as_posix(), recursive=True)
    with tarfile.open(archive) as source:
        for member in source:
            cache_validate_member(member, cache_scopes(component))
    return {'component': component, 'paths': [path.relative_to(root).as_posix() for path in paths],
            'archiveSeconds': time.monotonic() - started, 'archiveBytes': archive.stat().st_size,
            'format': 'local-uncompressed-tar'}


def cache_restore(root, component, definition, archive):
    import tarfile
    import time
    started = time.monotonic()
    if tuple(definition.build_paths(component)) != cache_scopes(component):
        raise ValueError('Unexpected native cache restore scope')
    # Validate the entire archive before extraction. Link targets are confined
    # to component roots, including Swift's literal relative .build aliases.
    with tarfile.open(archive) as source:
        members = source.getmembers()
        for member in members:
            cache_validate_member(member, cache_scopes(component))
            cache_guard_path(root, root / member.name)
        source.extractall(root, members=members, filter='data')
    return {'component': component, 'restoreSeconds': time.monotonic() - started,
            'archiveBytes': archive.stat().st_size, 'restoreSource': 'same-job-local-archive',
            'actionsCacheHit': None}


def cache_touch_tracked_inputs(root, components, definition):
    import hashlib
    import os
    import subprocess
    import time
    tracked = set(subprocess.check_output(['git', '-C', str(root), 'ls-files', '-z']).decode('utf8').split('\0'))
    paths = {path for component in components for path in definition.input_files(root, component)
             if path.relative_to(root).as_posix() in tracked}
    touched = 0
    for path in sorted(paths):
        cache_guard_path(root, path)
        if path.is_symlink() or not path.is_file():
            raise ValueError('Only regular tracked frozen inputs may simulate checkout mtimes')
        before = hashlib.sha256(path.read_bytes()).digest()
        stat = path.stat()
        os.utime(path, ns=(stat.st_atime_ns, time.time_ns()))
        if hashlib.sha256(path.read_bytes()).digest() != before:
            raise ValueError('Frozen input bytes changed during checkout simulation')
        touched += 1
    return touched


def cache_prepare(root, host, run_command, environment):
    import sys
    import time
    started = time.monotonic()
    phases = []
    if host == 'win32':
        arguments = ['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                     str(root / 'app/prepare-windows.ps1')]
        elapsed = run_command(arguments, cwd=root, env=environment, timeout=10800)
        phases.append({'phase': 'frozen-windows-preparation', 'seconds': elapsed})
    else:
        commands = [('macos-capture-core', ['sh', 'probes/macos-capture/build-core.sh']),
                    ('macos-capture-release', ['swift', 'build', '--package-path', 'probes/macos-capture', '--configuration', 'release']),
                    ('macos-capture-tests', ['swift', 'test', '--package-path', 'probes/macos-capture']),
                    ('macos-workers', [sys.executable, 'tools/mon_inference_runtime.py'])]
        for phase, arguments in commands:
            elapsed = run_command(arguments, cwd=root, env=environment, timeout=3600)
            phases.append({'phase': phase, 'seconds': elapsed})
    return time.monotonic() - started, phases


def cache_benchmark(root, report, checkpoint, run_command):
    import os
    import pathlib
    import statistics
    import sys
    import tempfile
    import time
    import mon_ci_cache as definition
    root = pathlib.Path(root).resolve()
    host = sys.platform
    if host not in ('win32', 'darwin'):
        raise ValueError('Native cache benchmark requires Windows or macOS')
    components = ['windows-runtime', 'windows-capture'] if host == 'win32' else ['macos-capture', 'macos-workers']
    environment = dict(os.environ)
    for name in ('MON_WINDOWS_PREPARATION_RECEIPT', 'MON_WINDOWS_CERTIFICATE_THUMBPRINT',
                 'MON_WINDOWS_TIMESTAMP_URL', 'MON_WINDOWS_SIGNTOOL', 'TAURI_CONFIG'):
        environment.pop(name, None)
    environment['MON_WINDOWS_RELEASE_SIGNING'] = '0'
    context = {'kind': 'same-job-intermediate-round-trip', 'actionsCacheHit': None,
               'simulatedCheckoutMtimes': True, 'components': components, 'trials': [],
               'comparisonMeaning': 'combined preparation reuse including retained fixed-download/dependency effects',
               'unsupportedInferences': ['isolated compilation-cache speedup', 'actual Actions cache service latency or hit'],
               'totalDefinition': 'local component restore plus preparation; excludes snapshot, cleanup and mtime simulation',
               'dependencyState': 'existing registry/npm/tool caches retained; fixed downloads retained or restored',
               'exclusions': ['Actions cache service/network restore time', 'installer/MSIX packaging', 'model weights'],
               'notes': ['Windows preparation includes frontend and notices; component timings cover archive/restore only',
                         'Swift capture trial includes release build and synthetic tests',
                         'Initial cold preparation includes any first-use fixed downloads and dependency setup; warm trials reuse them',
                         'Tracked source bytes and frozen commit are retained; only input mtimes are refreshed',
                         'Windows pinned Vulkan SDK extraction is removed and regenerated; capture headers are retained']}
    report['cache'] = context
    checkpoint()
    directory = pathlib.Path(os.environ['RUNNER_TEMP']).resolve()
    if not directory.is_dir() or directory == root or directory.is_relative_to(root):
        raise ValueError('Local benchmark archives require a separate runner temporary directory')
    with tempfile.TemporaryDirectory(prefix='mon-native-cache-round-trip-', dir=directory) as temporary:
        archives = {component: pathlib.Path(temporary) / (component + '.tar') for component in components}
        keys = {component: definition.cache_key(root, definition.toolchain_identity(component), component)
                for component in components}
        context['componentKeys'] = keys
        for index in range(4):
            row = {'trial': 'cold' if index == 0 else 'warm-' + str(index),
                   'status': 'running', 'actionsCacheHit': None, 'localIntermediateRestore': index != 0,
                   'intermediateState': 'removed before preparation' if index == 0 else 'restored from initial same-job snapshot',
                   'dependencyState': 'existing dependency caches retained as found' if index == 0 else 'dependency caches retained after initial preparation',
                   'fixedDownloadState': ('as found; declared macOS worker download cache cleared' if index == 0
                                          else 'Windows fixed downloads retained; macOS worker fixed downloads restored'),
                   'sdkExtractionState': 'pinned Windows SDK removed and regenerated' if host == 'win32' else 'not applicable',
                   'restoreComponents': [], 'totalSeconds': None}
            context['trials'].append(row)
            checkpoint()
            try:
                cache_remove_components(root, components, definition)
                row['regeneratedOutputs'] = cache_clean_generated(root, host)
                if index:
                    row['touchedTrackedInputs'] = cache_touch_tracked_inputs(root, components, definition)
                    for component in components:
                        row['restoreComponents'].append(cache_restore(root, component, definition, archives[component]))
                started = time.monotonic()
                row['prepareSeconds'], row['phases'] = cache_prepare(root, host, run_command, environment)
                row['totalSeconds'] = time.monotonic() - started + sum(item['restoreSeconds'] for item in row['restoreComponents'])
                if index == 0:
                    context['snapshots'] = [cache_snapshot(root, component, definition, archives[component])
                                            for component in components]
                current = {component: definition.cache_key(root, definition.toolchain_identity(component), component)
                           for component in components}
                if current != keys:
                    raise ValueError('Exact native cache key changed during benchmark')
                row['status'] = 'success'
            except BaseException as error:
                row['status'] = 'failure'
                row['failureType'] = type(error).__name__
                raise
            finally:
                checkpoint()
        warm = [row['totalSeconds'] for row in context['trials'][1:]]
        context['summary'] = {'coldSeconds': context['trials'][0]['totalSeconds'],
                              'warmMedianSeconds': statistics.median(warm),
                              'warmMinimumSeconds': min(warm), 'warmMaximumSeconds': max(warm)}
        checkpoint()


if __name__ == "__main__":
    raise SystemExit(main())
