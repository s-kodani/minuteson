"""Real worker protocol/log IO with fake inference; no model or user data."""
import ctypes
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile


def start(exe):
    process = subprocess.Popen([str(exe), "--mon-resident-v1"], stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8")
    send(process, {"id": 1, "op": "init"})
    assert receive(process)["op"] == "ready"
    return process


def send(process, value):
    process.stdin.write(json.dumps({"protocol": 1, **value}) + "\n")
    process.stdin.flush()


def receive(process):
    line = process.stdout.readline()
    assert line, "worker exited before replying"
    return json.loads(line)


def request(process, directory, index=1, stdout="stdout.log"):
    send(process, {"id": index, "op": "infer", "kind": "test", "args": [],
                   "directory": str(directory), "stdout": stdout, "stderr": "stderr.log"})
    while True:
        reply = receive(process)
        if reply["op"] != "progress":
            return reply


def stop(process):
    if process.poll() is None:
        try:
            send(process, {"id": 99, "op": "shutdown"})
        except BrokenPipeError:
            pass
    process.wait(timeout=10)
    process.stdin.close()
    process.stdout.close()
    process.stderr.close()


@contextmanager
def workspace():
    root = Path(tempfile.mkdtemp(prefix="mon-log-", dir=Path.cwd())).resolve()
    if os.name == "nt":
        root = Path("\\\\?\\" + str(root))
    try:
        yield root
    finally:
        shutil.rmtree(root)


def check(exe):
    # The current working directory is the owned CMake build/test directory.
    with workspace() as root:
        process = start(exe)
        try:
            for index in range(1, 9):
                prefix = f"日本語-{index}-"
                namespace = 4 if os.name == "nt" else 0
                unit = root / (prefix + "x" * max(1, 251 + namespace - len(str(root)) - 1 - len(prefix)))
                unit.mkdir()
                out = unit / "stdout.log"
                err = unit / "stderr.log"
                out.write_bytes(b"prefix\n")
                err.write_bytes(b"prefix\n")
                # Mimic the diagnostic collector following an existing log.
                with out.open("rb") as observer:
                    assert request(process, unit, index)["op"] == "done"
                    assert observer.read() == b"prefix\ntest unit completed\n"
                assert "using device protocol-test" in err.read_text(encoding="utf-8")
                assert len(str(out)) - namespace > 260
        finally:
            stop(process)

        unit = root / "missing-log"
        unit.mkdir()
        (unit / "stderr.log").write_bytes(b"")
        process = start(exe)
        try:
            reply = request(process, unit)
            assert reply["op"] == "error", reply
            assert not (unit / "stdout.log").exists()
            if os.name == "nt":
                assert "win32=2" in reply["message"], reply
        finally:
            stop(process)

        if os.name == "nt":
            from ctypes import wintypes
            kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
            kernel.CreateFileW.restype = wintypes.HANDLE
            kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            out = unit / "stdout.log"
            out.write_bytes(b"sealed\n")
            handle = kernel.CreateFileW(str(out), 0x80000000, 1, None, 3, 0, None)
            assert handle != ctypes.c_void_p(-1).value
            process = start(exe)
            try:
                reply = request(process, unit)
                assert reply["op"] == "error" and "win32=32" in reply["message"], reply
                assert out.read_bytes() == b"sealed\n"
            finally:
                stop(process)
                kernel.CloseHandle(handle)
    print("Eight long-path units, live log reader, missing log and Windows sharing checks passed.")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve(strict=True))
