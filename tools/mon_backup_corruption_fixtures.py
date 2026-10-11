#!/usr/bin/env python3
"""Create private, deliberately invalid QA copies; never restore or edit source.

The application remains the authority for semantic backup validation. This
helper checks framing/digests only so each copy fails for a known reason.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import struct
import tempfile

MAGIC = b"MONBAK01"
HEADER = 44
MAX_SOURCE = 64 * 1024 * 1024
MAX_MANIFEST = 8 * 1024 * 1024


def corruption_copies(body: bytes) -> dict[str, bytes]:
    if not HEADER < len(body) <= MAX_SOURCE or body[:8] != MAGIC:
        raise ValueError("source framing is not a bounded MON backup")
    length = struct.unpack_from("<I", body, 8)[0]
    if not 0 < length <= MAX_MANIFEST or HEADER + length >= len(body):
        raise ValueError("source manifest length is invalid")
    manifest_bytes = body[HEADER:HEADER + length]
    if hashlib.sha256(manifest_bytes).digest() != body[12:44]:
        raise ValueError("source manifest digest does not match")
    manifest = json.loads(manifest_bytes)
    entries = manifest.get("entries")
    if not isinstance(entries, list) or not entries:
        raise ValueError("source entries are missing")
    offset = HEADER + length
    for entry in entries:
        size = entry.get("size")
        if type(size) is not int or size < 1 or offset + size > len(body):
            raise ValueError("source entry length is invalid")
        payload = body[offset:offset + size]
        if hashlib.sha256(payload).hexdigest() != entry.get("sha256"):
            raise ValueError("source payload digest does not match")
        offset += size
    if offset != len(body):
        raise ValueError("source has trailing bytes")
    wrong_magic = b"NOTBAK01" + body[8:]
    digest = bytearray(body)
    digest[12] ^= 1
    payload = bytearray(body)
    payload[-1] ^= 1
    return {
        "wrong-magic.monbackup": wrong_magic,
        "truncated.monbackup": body[:-1],
        "manifest-digest.monbackup": bytes(digest),
        "payload-digest.monbackup": bytes(payload),
    }


def generate(source: Path, parent: Path) -> dict:
    # O_NOFOLLOW and stable fstat preserve the original even for invalid input.
    fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or not HEADER < before.st_size <= MAX_SOURCE:
            raise ValueError("source must be a regular backup of at most 64MiB")
        body = stream.read(MAX_SOURCE + 1)
        after = os.fstat(stream.fileno())
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
        ) or len(body) != before.st_size:
            raise ValueError("source changed while reading")
    copies = corruption_copies(body)
    root = Path(tempfile.mkdtemp(prefix="mon-backup-corruption-qa.", dir=parent))
    result = {"directory": str(root), "sourceBytes": len(body),
              "sourceSha256": hashlib.sha256(body).hexdigest(), "fixtures": []}
    for name, content in copies.items():
        destination = root / name
        with destination.open("xb") as output:
            os.chmod(destination, 0o600)
            output.write(content)
        result["fixtures"].append({"path": str(destination), "bytes": len(content),
                                   "sha256": hashlib.sha256(content).hexdigest()})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--parent", type=Path, default=Path(tempfile.gettempdir()))
    args = parser.parse_args()
    print(json.dumps(generate(args.source, args.parent), indent=2))


if __name__ == "__main__":
    main()
