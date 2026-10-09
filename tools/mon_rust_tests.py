#!/usr/bin/env python3
"""One bounded Cargo cache; build once, then run every selected test once.

macOS/Unix only. Production profiles, assertions, durable IO and ignored tests
are unchanged. The cache lock covers running tests, not just Cargo compilation.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
MARKER = ".mon-rust-tests.json"
MIB = 1024 * 1024
PRINT_LOCK = threading.Lock()


def report(message):
    with PRINT_LOCK:
        print(message, flush=True)


def footprint(path):
    """Allocated blocks, without following links or double-counting hardlinks."""
    total, seen = 0, set()
    pending = [path]
    while pending:
        current = pending.pop()
        metadata = current.lstat()
        identity = (metadata.st_dev, metadata.st_ino)
        if identity in seen:
            continue
        seen.add(identity)
        total += metadata.st_blocks * 512
        if stat.S_ISDIR(metadata.st_mode):
            pending.extend(current.iterdir())
    return total


class Cache:
    def __init__(self, path, repository=None):
        self.path = Path(os.path.abspath(path))
        self.repository = (ROOT if repository is None else repository).resolve()
        product = self.repository / "app" / "src-tauri" / "target"
        if (self.path == self.repository or self.path in self.repository.parents
                or self.path == Path.home().resolve() or self.path == Path("/")
                or self.path == product or product in self.path.parents
                or self.path in product.parents):
            raise ValueError("refusing a broad cache root")
        self.path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if self.path.parent.resolve() != self.path.parent:
            raise ValueError("cache ancestors must not be symlinks")
        self.identity = None
        self.lock_fd = None

    @contextmanager
    def locked(self):
        lock_path = self.path.with_name(self.path.name + ".lock")
        fd = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            metadata = os.fstat(fd)
            if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid()
                    or metadata.st_nlink != 1 or metadata.st_mode & 0o077):
                raise ValueError("unsafe cache lock")
            report("[rust-tests] Waiting for the shared test-cache lock")
            fcntl.flock(fd, fcntl.LOCK_EX)
            self.prepare()
            self.lock_fd = fd
            yield self
        finally:
            self.lock_fd = None
            os.close(fd)

    def prepare(self):
        self.path.mkdir(mode=0o700, exist_ok=True)
        metadata = self.path.lstat()
        if (not stat.S_ISDIR(metadata.st_mode) or self.path.resolve() != self.path
                or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077):
            raise ValueError("unsafe cache directory")
        self.identity = (metadata.st_dev, metadata.st_ino)
        expected = {"schemaVersion": 1, "repository": str(self.repository)}
        marker = self.path / MARKER
        if not marker.exists():
            if any(self.path.iterdir()):
                raise ValueError("refusing an unowned, nonempty cache")
            fd = os.open(marker, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, "w") as output:
                json.dump(expected, output)
        self.verify()

    def verify(self):
        metadata = self.path.lstat()
        if (not stat.S_ISDIR(metadata.st_mode) or self.path.resolve() != self.path
                or (metadata.st_dev, metadata.st_ino) != self.identity
                or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077):
            raise ValueError("cache identity changed")
        fd = os.open(self.path / MARKER, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(fd, "r") as source:
            proof = os.fstat(source.fileno())
            if (not stat.S_ISREG(proof.st_mode) or proof.st_size > 4096
                    or proof.st_uid != os.geteuid() or proof.st_nlink != 1):
                raise ValueError("unsafe cache ownership marker")
            value = json.load(source)
        if value != {"schemaVersion": 1, "repository": str(self.repository)}:
            raise ValueError("cache belongs to a different repository")

    def remove_debug(self, incremental_only, incremental_entry=None, incremental_session=None):
        self.verify()
        # rmtree's descriptor-relative implementation does not follow replaced
        # symlinks. Never clean a user-supplied Cargo target or production target.
        if not shutil.rmtree.avoids_symlink_attacks:
            raise ValueError("safe descriptor-relative cleanup is unavailable")
        root_fd = os.open(self.path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            metadata = os.fstat(root_fd)
            if (metadata.st_dev, metadata.st_ino) != self.identity:
                raise ValueError("cache was replaced before cleanup")
            try:
                debug_fd = os.open("debug", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                   dir_fd=root_fd)
            except FileNotFoundError:
                return
            try:
                if incremental_only:
                    try:
                        metadata = os.stat("incremental", dir_fd=debug_fd, follow_symlinks=False)
                    except FileNotFoundError:
                        return
                    if not stat.S_ISDIR(metadata.st_mode):
                        raise ValueError("incremental cache is not a real directory")
                    if incremental_entry is None:
                        shutil.rmtree("incremental", dir_fd=debug_fd)
                    else:
                        if (incremental_entry in {"", ".", ".."}
                                or Path(incremental_entry).name != incremental_entry):
                            raise ValueError("invalid incremental cache entry")
                        inc_fd = os.open("incremental", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                         dir_fd=debug_fd)
                        try:
                            if incremental_session is None:
                                shutil.rmtree(incremental_entry, dir_fd=inc_fd)
                            else:
                                if (Path(incremental_session).name != incremental_session
                                        or not incremental_session.startswith("s-")):
                                    raise ValueError("invalid incremental session")
                                crate_fd = os.open(incremental_entry, os.O_RDONLY | os.O_DIRECTORY
                                                   | os.O_NOFOLLOW, dir_fd=inc_fd)
                                try:
                                    shutil.rmtree(incremental_session, dir_fd=crate_fd)
                                finally:
                                    os.close(crate_fd)
                        finally:
                            os.close(inc_fd)
                else:
                    shutil.rmtree("debug", dir_fd=root_fd)
            finally:
                os.close(debug_fd)
        finally:
            os.close(root_fd)

    def trim(self, maximum):
        self.verify()
        before = footprint(self.path)
        if before > maximum:
            incremental = self.path / "debug" / "incremental"
            if incremental.exists():
                if incremental.is_symlink() or not incremental.is_dir():
                    raise ValueError("unsafe incremental cache")
                candidates = sorted((child for child in incremental.iterdir()
                                     if child.is_dir() and not child.is_symlink()),
                                    key=lambda child: child.stat().st_mtime_ns)
                # Rust retains multiple generations inside each crate. Keep
                # its newest completed generation before evicting whole crates.
                # All compilation/testing children have exited at this point.
                for crate in candidates:
                    sessions = sorted((child for child in crate.iterdir()
                                       if child.name.startswith("s-") and child.is_dir()
                                       and not child.is_symlink()),
                                      key=lambda child: child.stat().st_mtime_ns)
                    completed = [child for child in sessions if not child.name.endswith("-working")]
                    stale = [child for child in sessions if child not in completed[-1:]]
                    for child in stale:
                        if footprint(self.path) <= maximum:
                            break
                        self.remove_debug(True, crate.name, child.name)
                for child in candidates:
                    if footprint(self.path) <= maximum:
                        break
                    self.remove_debug(incremental_only=True, incremental_entry=child.name)
            if footprint(self.path) > maximum:
                self.remove_debug(incremental_only=True)
        if footprint(self.path) > maximum:
            self.remove_debug(incremental_only=False)
        after = footprint(self.path)
        if after > maximum:
            raise ValueError("non-build files exceed the cache budget; retained for inspection")
        if after < before:
            report(f"[rust-tests] Reclaimed {(before-after)/MIB:.1f} MiB of rebuildable cache")
        return after


def inventory(output):
    names = [line[:-6] for line in output.splitlines() if line.endswith(": test")]
    if len(names) != len(set(names)) or len(names) > 25_000:
        raise ValueError("invalid test inventory")
    if not re.search(r"^\d+ tests?, \d+ benchmarks?$", output, re.MULTILINE):
        raise ValueError("missing test inventory summary")
    summary = re.search(r"^(\d+) tests?, (\d+) benchmarks?$", output, re.MULTILINE)
    if int(summary[1]) != len(names) or int(summary[2]) != 0:
        raise ValueError("inventory mismatch or unsupported benchmark target")
    return names


def batches(names, maximum=24 * 1024):
    """Exact names, bounded argv size, and long common-module runs interleaved."""
    # Round-robin by module avoids queuing all serialized history tests in one
    # process while another process has only small CPU-only tests.
    modules = {}
    for name in sorted(names):
        modules.setdefault(name.rsplit("::", 1)[0], []).append(name)
    ordered = []
    while modules:
        for module in list(modules):
            ordered.append(modules[module].pop(0))
            if not modules[module]:
                del modules[module]
    result, current, size = [], [], 0
    for name in ordered:
        length = len(name.encode()) + 1
        if length > maximum:
            raise ValueError("test name exceeds argument budget")
        if current and (size + length > maximum or len(current) >= 64):
            result.append(current)
            current, size = [], 0
        current.append(name)
        size += length
    if current:
        result.append(current)
    if sorted(name for batch in result for name in batch) != sorted(names):
        raise ValueError("test partition lost or duplicated a test")
    return result


def build(command, environment, cwd, lock_fd=None):
    executables = []
    child = subprocess.Popen(command, env=environment, cwd=cwd, stdout=subprocess.PIPE,
                             text=True, pass_fds=() if lock_fd is None else (lock_fd,))
    try:
        for line in child.stdout:
            value = json.loads(line)
            if value.get("reason") == "compiler-message":
                report(value["message"].get("rendered", "").rstrip())
            if (value.get("reason") == "compiler-artifact" and value.get("executable")
                    and value.get("profile", {}).get("test")):
                executable = Path(value["executable"])
                if executable not in executables:
                    executables.append(executable)
    finally:
        child.stdout.close()
        code = child.wait()
    return code, executables


def run_units(executable, cwd, environment, processes, threads, selected, extra, lock_fd=None):
    inherited = () if lock_fd is None else (lock_fd,)
    if selected is not None or extra:
        arguments = ([] if selected is None else [selected]) + extra
        return subprocess.run([str(executable), *arguments, f"--test-threads={threads}"],
                              cwd=cwd, env=environment, pass_fds=inherited).returncode
    listed = subprocess.run([str(executable), "--list"], cwd=cwd, env=environment,
                            capture_output=True, text=True, timeout=30, check=True,
                            pass_fds=inherited)
    names = inventory(listed.stdout)
    if processes == 1:
        # Keep the full libtest scheduler, without per-batch barriers. Sharding
        # is opt-in until a full-suite comparison proves a benefit here.
        return subprocess.run([str(executable), f"--test-threads={threads}"],
                              cwd=cwd, env=environment, pass_fds=inherited).returncode
    groups = batches(names) or [[]]
    report(f"[rust-tests] {len(names)} inventoried tests; {len(groups)} exact batches; "
           f"at most {processes} processes x {threads} threads")
    failed = threading.Event()

    def execute(group):
        if failed.is_set():
            return 1
        code = subprocess.run([str(executable), *group, "--exact", f"--test-threads={threads}"],
                              cwd=cwd, env=environment, pass_fds=inherited).returncode
        if code:
            failed.set()
        return code

    with ThreadPoolExecutor(max_workers=processes) as pool:
        codes = list(pool.map(execute, groups))
    return next((code for code in codes if code), 0)


def main(argv=None):
    arguments = sys.argv[1:] if argv is None else argv
    split = arguments.index("--") if "--" in arguments else len(arguments)
    extra = arguments[split+1:] if split < len(arguments) else []
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("filter", nargs="?")
    parser.add_argument("--cache-dir", type=Path, default=ROOT / ".cache" / "rust-tests")
    parser.add_argument("--max-cache-mib", type=int, default=10240)
    parser.add_argument("--min-free-mib", type=int, default=2048)
    parser.add_argument("--processes", type=int, choices=range(1, 5), default=1)
    parser.add_argument("--jobs", type=int, choices=range(1, 17),
                        default=min(4, os.cpu_count() or 1))
    parser.add_argument("--threads", type=int, choices=range(1, 17),
                        default=min(16, os.cpu_count() or 1))
    parser.add_argument("--build-only", action="store_true")
    parser.add_argument("--status", action="store_true")
    parser.add_argument("--clean", action="store_true")
    options = parser.parse_args(arguments[:split])
    if options.filter == "":
        parser.error("focused checks require a nonempty test filter")
    if options.max_cache_mib < 1 or options.min_free_mib < 0:
        parser.error("cache budget must be positive; free-space floor must be nonnegative")
    cache = Cache(options.cache_dir)
    with cache.locked():
        maximum = options.max_cache_mib * MIB
        if options.clean:
            cache.remove_debug(incremental_only=False)
        size = footprint(cache.path) if options.status else cache.trim(maximum)
        report(f"[rust-tests] Cache {cache.path}: {size/MIB:.1f} MiB; "
               f"post-run budget {options.max_cache_mib} MiB")
        if options.status or options.clean:
            return 0
        if shutil.disk_usage(cache.path).free < options.min_free_mib * MIB:
            raise ValueError("insufficient free space before build; no tests started")
        environment = os.environ.copy()
        environment["CARGO_TARGET_DIR"] = str(cache.path)
        manifest = ROOT / "app" / "src-tauri" / "Cargo.toml"
        common = ["cargo", "test", "--locked", "--offline", "--jobs", str(options.jobs),
                  "--manifest-path", str(manifest)]
        focused = options.filter is not None or bool(extra)
        command = common + (["--lib"] if focused else [])
        started = time.monotonic()
        built, code = started, 1
        finished = False
        try:
            build_code, executables = build(command + ["--no-run", "--message-format=json"],
                                            environment, manifest.parent, cache.lock_fd)
            built = time.monotonic()
            code = build_code
            if not code and not executables:
                code = 1
                raise ValueError("Cargo produced no test executable")
            if not code and not options.build_only:
                for executable in executables:
                    code = run_units(executable, manifest.parent, environment,
                                     options.processes, options.threads, options.filter, extra,
                                     cache.lock_fd)
                    if code:
                        break
                if not code and not focused:
                    # Cargo --no-run excludes doc tests. Preserve full-check
                    # coverage, including future binary/integration test targets.
                    code = subprocess.run(common + ["--doc"], cwd=manifest.parent,
                                          env=environment, pass_fds=(cache.lock_fd,)).returncode
            finished = True
        finally:
            elapsed = time.monotonic() - built
            # subprocess.run can raise KeyboardInterrupt before its child has
            # exited. The inherited lock keeps later runners out; never trim
            # underneath a possibly live Cargo/libtest/doc-test child.
            size = cache.trim(maximum) if finished else footprint(cache.path)
            if not finished:
                report("[rust-tests] Interrupted workflow: cache retained until a later locked run")
            report(f"[rust-tests] Build {built-started:.2f}s; execution {elapsed:.2f}s; "
                   f"cache {size/MIB:.1f} MiB; exit {code}")
        return code


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"[rust-tests] {error}", file=sys.stderr)
        sys.exit(1)
