"""Build-time, version-pinned CPU runtime; never run this on an end user's Mac.

Only a small sealed TAR and its index go into MON. Model weights remain a
separate, explicitly confirmed acquisition. No app/user data or global tool
installation is touched. A cached, fully checked package avoids recompilation.
"""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import time

REPOSITORY = Path(__file__).resolve().parents[1]
COMMIT = "927cfce34f31707e17f2bff35c349632fb9e2c3a"
VERSION = "v1.9.4"
SOURCE_URL = f"https://codeload.github.com/ggml-org/whisper.cpp/tar.gz/{COMMIT}"
SOURCE_BYTES = 9357755
# Independently calculated from the acquired official archive, not a signature.
SOURCE_SHA = "41b664fee09e79176ac277b5237debec34f8d74af3c7d71f333f1ec67989ecde"
SOURCE_ROOT = f"whisper.cpp-{COMMIT}"
MAX_EXPANDED = 128 * 1024 ** 2
MAX_ENTRIES = 10000
MAX_PACKAGE = 16 * 1024 ** 2
MEMBERS = ("bin/whisper-cli", "notices/whisper-MIT.txt", "build-info.json")
OPTIONS = (
    "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_SHARED_LIBS=OFF",
    "-DCMAKE_C_COMPILER=/usr/bin/clang", "-DCMAKE_CXX_COMPILER=/usr/bin/clang++",
    "-DCMAKE_OSX_ARCHITECTURES=arm64", "-DCMAKE_OSX_DEPLOYMENT_TARGET=15.0",
    "-DGGML_NATIVE=OFF", "-DGGML_CPU_ARM_ARCH=armv8-a",
    "-DGGML_METAL=OFF", "-DGGML_OPENMP=OFF", "-DGGML_ACCELERATE=ON",
    "-DGGML_BLAS=ON", "-DGGML_BLAS_VENDOR=Apple", "-DGGML_CPU_KLEIDIAI=OFF",
    "-DGGML_BACKEND_DL=OFF", "-DWHISPER_COREML=OFF", "-DWHISPER_OPENVINO=OFF",
    "-DWHISPER_BUILD_TESTS=OFF", "-DWHISPER_BUILD_EXAMPLES=ON",
    "-DFETCHCONTENT_FULLY_DISCONNECTED=ON",
)


def private(path, *, directory=False):
    if not path.is_absolute() or path.resolve() != path or path.is_symlink():
        raise ValueError("runtime path must be absolute and have no linked parents")
    info = path.stat()
    if not (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode)):
        raise ValueError("runtime path has the wrong file type")
    return path


def digest(path):
    private(path)
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def check_source(path):
    private(path)
    if path.stat().st_size != SOURCE_BYTES or digest(path) != SOURCE_SHA:
        raise ValueError("official source archive size/SHA differs from the pinned definition")


def environment(cmake):
    # Do not carry signing secrets, proxy/TLS overrides, DYLD or caller CFLAGS.
    return {"PATH": f"{cmake.parent}:/usr/bin:/bin:/usr/sbin:/sbin", "LC_ALL": "C"}


def command(argv, *, env=None, timeout=600):
    if env is None:
        env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "LC_ALL": "C"}
    process = subprocess.Popen(argv, env=env, start_new_session=True,
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt):
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
        raise
    if len(stdout) + len(stderr) > 4 * 1024 ** 2:
        raise ValueError("runtime tool output exceeded the diagnostic limit")
    if process.returncode:
        details = stderr.decode("utf8", errors="replace")[-8192:]
        raise ValueError(f"{Path(argv[0]).name} failed (exit {process.returncode}); no runtime published: {details}")
    return stdout, stderr


def acquire(destination, supplied):
    if destination.exists() or destination.is_symlink():
        check_source(destination)
        return
    # The temporary directory is ours and create-new; unknown files are never
    # deleted/reused. Completed source is checked before publication or builds.
    with tempfile.TemporaryDirectory(prefix="source-", dir=destination.parent) as temporary:
        pending = Path(temporary) / "source.tar.gz"
        if supplied is not None:
            check_source(supplied)
            with supplied.open("rb") as source, pending.open("xb") as output:
                remaining = SOURCE_BYTES + 1
                while remaining:
                    block = source.read(min(remaining, 1024 * 1024))
                    if not block:
                        break
                    output.write(block)
                    remaining -= len(block)
        else:
            download_source(["/usr/bin/curl", "--disable", "--globoff", "--silent", "--show-error",
                     "--fail", "--proto", "=https", "--proto-redir", "=https", "--location",
                     "--max-redirs", "3", "--connect-timeout", "15", "--max-time", "120",
                     "--max-filesize", str(SOURCE_BYTES), "--url", SOURCE_URL], pending, SOURCE_BYTES)
        check_source(pending)
        pending.chmod(0o600)
        os.rename(pending, destination)


def download_source(argv, pending, expected_size, timeout=130):
    """Parent-side byte bound also covers chunked bodies without Content-Length."""
    process = subprocess.Popen(argv, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
                               start_new_session=True, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    deadline, received, error = time.monotonic() + timeout, 0, bytearray()
    try:
        with selectors.DefaultSelector() as selector, pending.open("xb") as output:
            for stream in (process.stdout, process.stderr):
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_READ)
            while selector.get_map():
                if time.monotonic() >= deadline:
                    raise ValueError("source download exceeded its time limit")
                for event, _ in selector.select(min(0.25, max(0, deadline - time.monotonic()))):
                    block = os.read(event.fileobj.fileno(), 65536)
                    if not block:
                        selector.unregister(event.fileobj)
                        continue
                    if event.fileobj is process.stdout:
                        received += len(block)
                        if received > expected_size:
                            raise ValueError("source download exceeded its parent-side byte limit")
                        output.write(block)
                    else:
                        error.extend(block)
                        if len(error) > 8192:
                            raise ValueError("source transport diagnostics exceeded their limit")
            if process.wait(timeout=5) != 0 or received != expected_size:
                raise ValueError("source download failed or returned an unexpected capacity")
    finally:
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
        process.stdout.close()
        process.stderr.close()


def extract_source(archive, destination):
    private(archive)
    private(destination, directory=True)
    if any(destination.iterdir()):
        raise ValueError("source extraction destination must be empty")
    with archive.open("rb") as input_archive:
        identity = os.fstat(input_archive.fileno())
        if identity.st_size != SOURCE_BYTES:
            raise ValueError("source archive capacity differs")
        hasher = hashlib.sha256()
        for block in iter(lambda: input_archive.read(1024 * 1024), b""):
            hasher.update(block)
        if hasher.hexdigest() != SOURCE_SHA:
            raise ValueError("source archive hash differs before extraction")
        input_archive.seek(0)
        with tarfile.open(fileobj=input_archive, mode="r:gz") as source:
            extract_members(source, destination)
        fields = lambda info: (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        if fields(os.fstat(input_archive.fileno())) != fields(identity) or fields(archive.stat()) != fields(identity) or archive.is_symlink():
            raise ValueError("source archive changed during extraction")
    root = destination / SOURCE_ROOT
    private(root / "CMakeLists.txt")
    private(root / "LICENSE")
    return root


def extract_members(source, destination):
    seen, total = set(), 0
    for index, member in enumerate(source):
        path = PurePosixPath(member.name)
        if (index >= MAX_ENTRIES or member.name != path.as_posix()
                or path.is_absolute() or not path.parts or path.parts[0] != SOURCE_ROOT
                or any(part in (".", "..") for part in path.parts)
                or "\\" in member.name or ":" in member.name
                or any(ord(c) < 32 or ord(c) == 127 for c in member.name)
                or member.name.lower() in seen or not (member.isdir() or member.isfile())
                or member.size < 0):
            raise ValueError("unsafe, duplicate or excessive source archive entry")
        seen.add(member.name.lower())
        target = destination.joinpath(*path.parts)
        if member.isdir():
            target.mkdir(mode=0o700, parents=True, exist_ok=True)
            continue
        total += member.size
        if total > MAX_EXPANDED:
            raise ValueError("source archive expansion exceeds 128MiB")
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        with source.extractfile(member) as input_file, target.open("xb") as output:
            remaining = member.size
            while remaining:
                block = input_file.read(min(remaining, 1024 * 1024))
                if not block:
                    raise ValueError("truncated source archive")
                output.write(block)
                remaining -= len(block)
        target.chmod(0o600)


def verify_binary(binary):
    private(binary)
    if not 0 < binary.stat().st_size <= MAX_PACKAGE or not binary.stat().st_mode & 0o111:
        raise ValueError("runtime binary size or execute permission is invalid")
    arches, _ = command(["/usr/bin/lipo", "-archs", str(binary)], timeout=30)
    if arches.strip() != b"arm64":
        raise ValueError("runtime must be arm64 only")
    dependencies, _ = command(["/usr/bin/otool", "-L", str(binary)], timeout=30)
    paths = [line.strip().split(" (", 1)[0] for line in dependencies.decode().splitlines()[1:]]
    allowed = {"/usr/lib/libc++.1.dylib", "/usr/lib/libSystem.B.dylib",
               "/System/Library/Frameworks/Accelerate.framework/Versions/A/Accelerate"}
    if not paths or not set(paths) <= allowed:
        raise ValueError("runtime has non-system or unexpected dependencies")
    load_commands, _ = command(["/usr/bin/otool", "-l", str(binary)], timeout=30)
    import re
    versions = re.findall(r"\bminos\s+([0-9.]+)", load_commands.decode())
    if versions != ["15.0"]:
        raise ValueError("runtime minimum macOS must be exactly 15.0")
    command(["/usr/bin/codesign", "--verify", "--strict", str(binary)], timeout=30)


def build_info():
    return {"schemaVersion": 1, "engine": "whisper.cpp", "engineVersion": VERSION,
            "sourceCommit": COMMIT, "sourceUrl": SOURCE_URL, "sourceBytes": SOURCE_BYTES,
            "sourceSha256": SOURCE_SHA, "target": "macos_aarch64", "minimumMacOS": "15.0",
            "buildOptions": list(OPTIONS), "cpuOnly": True,
            "publisherAuthenticity": "not_verified", "licenseReview": "not_verified",
            "distributionReady": False}


def package(binary, license_file, destination):
    private(destination, directory=True)
    if any(destination.iterdir()):
        raise ValueError("runtime package destination must be empty")
    private(binary)
    private(license_file)
    if not 0 < binary.stat().st_size <= MAX_PACKAGE or not 0 < license_file.stat().st_size <= 65536:
        raise ValueError("runtime binary/license size exceeds the package limit")
    info_bytes = json.dumps(build_info(), separators=(",", ":"), ensure_ascii=False).encode()
    files = {"bin/whisper-cli": binary.read_bytes(), "notices/whisper-MIT.txt": license_file.read_bytes(),
             "build-info.json": info_bytes}
    if len(files["notices/whisper-MIT.txt"]) > 65536 or not files["notices/whisper-MIT.txt"]:
        raise ValueError("runtime license body is empty or too large")
    archive = destination / "whisper-runtime.tar"
    entries = []
    with tarfile.open(archive, "x", format=tarfile.USTAR_FORMAT) as output:
        for name in MEMBERS:
            content = files[name]
            member = tarfile.TarInfo(name)
            member.size, member.mode, member.mtime = len(content), 0o600, 0
            output.addfile(member, io.BytesIO(content))
            entries.append({"relativePath": name, "bytes": len(content),
                            "sha256": hashlib.sha256(content).hexdigest()})
    index = {**build_info(), "archive": {"file": archive.name, "bytes": archive.stat().st_size,
             "sha256": digest(archive)}, "files": entries, "runtimeSigning": "ad_hoc_only"}
    with (destination / "runtime-index.json").open("x", encoding="utf8") as output:
        json.dump(index, output, ensure_ascii=False, separators=(",", ":"))
    verify_package(destination)
    return index


def verify_package(directory):
    private(directory, directory=True)
    if {path.name for path in directory.iterdir()} != {"runtime-index.json", "whisper-runtime.tar"}:
        raise ValueError("runtime package has missing or unknown files")
    index_path = private(directory / "runtime-index.json")
    if not 0 < index_path.stat().st_size <= 65536:
        raise ValueError("runtime index exceeds the metadata limit")
    index = json.loads(index_path.read_text(encoding="utf8"))
    expected = build_info()
    if (not isinstance(index, dict) or set(index) != set(expected) | {"archive", "files", "runtimeSigning"}
            or any(json.dumps(index.get(key), sort_keys=True) != json.dumps(value, sort_keys=True) for key, value in expected.items())
            or index.get("runtimeSigning") != "ad_hoc_only"):
        raise ValueError("runtime index differs from the fixed CPU build definition")
    archive = private(directory / "whisper-runtime.tar")
    if (not 0 < archive.stat().st_size <= MAX_PACKAGE
            or index["archive"] != {"file": archive.name, "bytes": archive.stat().st_size, "sha256": digest(archive)}):
        raise ValueError("runtime archive size/SHA does not match its index")
    if not isinstance(index["files"], list) or len(index["files"]) != len(MEMBERS):
        raise ValueError("runtime package member list differs")
    with tarfile.open(archive, "r:") as source:
        count = 0
        for position, member in enumerate(source):
            if position >= len(MEMBERS) or member.name != MEMBERS[position] or not member.isfile():
                raise ValueError("runtime archive has unsafe or unknown entries")
            content = source.extractfile(member).read(MAX_PACKAGE + 1)
            if not 0 < len(content) == member.size <= MAX_PACKAGE:
                raise ValueError("runtime archive member size differs")
            if index["files"][position] != {"relativePath": member.name, "bytes": len(content),
                                          "sha256": hashlib.sha256(content).hexdigest()}:
                raise ValueError("runtime archive member SHA differs")
            if member.name == "build-info.json" and json.dumps(json.loads(content), sort_keys=True) != json.dumps(expected, sort_keys=True):
                raise ValueError("runtime build provenance differs")
            count += 1
        if count != len(MEMBERS):
            raise ValueError("runtime archive member is missing")
    return index


@contextmanager
def lock(cache):
    path = cache / "prepare.lock"
    descriptor = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise ValueError("runtime preparation lock must be a regular file")
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield
    finally:
        os.close(descriptor)


def prepare(cache, supplied, cmake, jobs):
    if sys.platform != "darwin" or platform.machine() != "arm64":
        raise ValueError("ASR runtime preparation requires an Apple Silicon build host")
    if not cache.is_absolute() or cache.is_symlink() or cache.resolve() != cache:
        raise ValueError("runtime cache must be an absolute, non-linked directory")
    if not cache.exists():
        if cache.parent == REPOSITORY / ".cache" and not cache.parent.exists():
            private(REPOSITORY, directory=True)
            cache.parent.mkdir(mode=0o700)
        private(cache.parent, directory=True)
        cache.mkdir(mode=0o700)
    private(cache, directory=True)
    with lock(cache):
        destination = cache / "package"
        if destination.exists() or destination.is_symlink():
            index = verify_package(destination)
            return {"state": "checked_cached_package", "package": str(destination),
                    "archiveBytes": index["archive"]["bytes"], "networkBytes": 0}
        if shutil.disk_usage(cache).free < 384 * 1024 ** 2:
            raise ValueError("at least 384MiB free is needed for transient ASR compilation")
        if not 1 <= jobs <= 4:
            raise ValueError("ASR build parallelism must be 1..4")
        if cmake is None:
            found = shutil.which("cmake")
            if not found:
                raise ValueError("build host needs CMake or MON_WHISPER_CMAKE; end users do not")
            cmake = Path(found).resolve()
        private(cmake)
        source_archive = cache / "whisper-source.tar.gz"
        had_source = source_archive.exists()
        acquire(source_archive, supplied)
        started = time.monotonic()
        # Only this newly owned directory is reclaimed, including failures.
        # Source and reusable sealed package together use roughly 12MiB.
        with tempfile.TemporaryDirectory(prefix="compile-", dir=cache) as temporary:
            work = Path(temporary)
            source_dir = work / "source"
            source_dir.mkdir(mode=0o700)
            source = extract_source(source_archive, source_dir)
            build_dir = work / "build"
            clean = environment(cmake)
            command([str(cmake), "-S", str(source), "-B", str(build_dir), *OPTIONS], env=clean)
            command([str(cmake), "--build", str(build_dir), "--target", "whisper-cli", "--parallel", str(jobs)], env=clean)
            binary = private(build_dir / "bin/whisper-cli")
            command(["/usr/bin/codesign", "--force", "--sign", "-", "--identifier", "jp.mon.desktop.whisper-runtime", str(binary)], env=clean, timeout=30)
            verify_binary(binary)
            pending = work / "package"
            pending.mkdir(mode=0o700)
            index = package(binary, source / "LICENSE", pending)
            os.rename(pending, destination)
        return {"state": "built_checked_package", "package": str(destination),
                "archiveBytes": index["archive"]["bytes"],
                "networkBytes": 0 if supplied is not None or had_source else SOURCE_BYTES,
                "compileAndPackageSeconds": round(time.monotonic() - started, 3)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=REPOSITORY / ".cache/whisper-runtime")
    parser.add_argument("--source-archive", type=Path, default=os.environ.get("MON_WHISPER_SOURCE_ARCHIVE"))
    parser.add_argument("--cmake", type=Path, default=os.environ.get("MON_WHISPER_CMAKE"))
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--verify-package", type=Path)
    args = parser.parse_args(argv)
    try:
        result = verify_package(args.verify_package) if args.verify_package else prepare(args.cache_dir, args.source_archive, args.cmake, args.jobs)
        print(json.dumps(result, ensure_ascii=False))
        return 0
    except (ValueError, OSError, tarfile.TarError, subprocess.TimeoutExpired) as error:
        print(f"ASR runtime preparation failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
