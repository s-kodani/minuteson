#!/usr/bin/env python3
"""Bind a Windows installer to its source, app and capture helper.

This tool never installs or launches binaries. `finish` refuses a build whose
source changed; `verify` also accepts explicitly supplied installed executables.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE_DIRECTORIES = ("app", "crates", ".cargo", "probes", "tools", "third-party", ".github/workflows")
SOURCE_FILES = ("Cargo.toml", "Cargo.lock")
SOURCE_PATHS = (*SOURCE_DIRECTORIES, *SOURCE_FILES)


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def sha256_file(path):
    with Path(path).open("rb") as source:
        before = os.fstat(source.fileno())
        digest = hashlib.file_digest(source, "sha256").hexdigest()
        after = os.fstat(source.fileno())
        current = Path(path).stat()
        def identity(stat):
            # Windows fstat reports metadata-change time in st_ctime, while
            # path stat can still report creation time there. Birth time is
            # consistent across both APIs; retain ctime for the same open fd.
            stamp = stat.st_birthtime_ns if os.name == "nt" else stat.st_ctime_ns
            return (stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stamp)
        if (identity(before) != identity(after) or before.st_ctime_ns != after.st_ctime_ns
                or identity(after) != identity(current)):
            raise ValueError(f"File changed while calculating SHA-256: {path}")
    return digest


def source_identity(root):
    commit = git(root, "rev-parse", "HEAD").decode().strip()
    patch = git(root, "diff", "--binary", "--no-ext-diff", "HEAD", "--", *SOURCE_PATHS)
    names = git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *SOURCE_PATHS)
    entries = []
    for name in sorted(set(names.decode("utf-8").split("\0")) - {""}):
        path = root / name
        if path.is_symlink() or (path.exists() and not path.resolve().is_relative_to(root)):
            raise ValueError(f"Build source must stay inside the workspace: {name}")
        if path.exists():
            entries.append({"path": name, "sha256": sha256_file(path)})
        else:
            entries.append({"path": name, "deleted": True})
    encoded = json.dumps(entries, ensure_ascii=True, sort_keys=True, separators=(",", ":")).encode()
    return {
        "commit": commit,
        "patchSha256": hashlib.sha256(patch).hexdigest(),
        "sourceTreeSha256": hashlib.sha256(encoded).hexdigest(),
        "files": entries,
    }


def validate_source_identity(source):
    def digest(value):
        return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None
    if not isinstance(source, dict) or set(source) != {"commit", "patchSha256", "sourceTreeSha256", "files"}:
        raise ValueError("Build source identity is missing or invalid")
    if (not isinstance(source["commit"], str) or re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", source["commit"]) is None
            or not digest(source["patchSha256"]) or not digest(source["sourceTreeSha256"]) or not isinstance(source["files"], list)):
        raise ValueError("Build source identity is malformed")
    names = []
    for entry in source["files"]:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
            raise ValueError("Build source file identity is malformed")
        name = entry["path"]
        if (not (name in SOURCE_FILES or any(name.startswith(prefix + "/") for prefix in SOURCE_DIRECTORIES))
                or any(part in ("", ".", "..") for part in name.split("/")) or "\\" in name
                or not ((set(entry) == {"path", "sha256"} and digest(entry["sha256"]))
                        or (set(entry) == {"path", "deleted"} and entry["deleted"] is True))):
            raise ValueError("Build source file identity is malformed")
        names.append(name)
    encoded = json.dumps(source["files"], ensure_ascii=True, sort_keys=True, separators=(",", ":")).encode()
    if names != sorted(set(names)) or hashlib.sha256(encoded).hexdigest() != source["sourceTreeSha256"]:
        raise ValueError("Build source tree identity does not match its files")


def workspace_path(root, path):
    path = Path(path).absolute()
    if not path.is_relative_to(root) or path == root:
        raise ValueError("Build evidence and artifacts must stay inside the workspace")
    # Reject links/junctions in existing ancestors, including a linked file.
    if path.resolve() != path:
        raise ValueError("Build evidence cannot follow a link")
    return path


def save_new(root, path, value):
    path = workspace_path(root, path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(value, output, ensure_ascii=True, indent=2)
        output.write("\n")


def executable(root, path):
    path = workspace_path(root, path)
    with path.open("rb") as source:
        if source.read(2) != b"MZ":
            raise ValueError(f"Not a Windows executable: {path}")
    return {"path": path.relative_to(root).as_posix(), "bytes": path.stat().st_size, "sha256": sha256_file(path)}


def begin(root, output):
    save_new(root, output, {"schemaVersion": 1, "startedAt": utc_now(), "source": source_identity(root)})


def finish(root, snapshot, app, helper, installer, output):
    snapshot = json.loads(workspace_path(root, snapshot).read_text(encoding="utf-8"))
    if snapshot.get("schemaVersion") != 1 or snapshot["source"] != source_identity(root):
        raise ValueError("Build source changed; retain the previous installer and rebuild")
    manifest = {
        **snapshot,
        "completedAt": utc_now(),
        "artifacts": {name: executable(root, path) for name, path in (("app", app), ("helper", helper), ("installer", installer))},
        "installedVerification": "pending",
    }
    save_new(root, output, manifest)


def verify(root, manifest_path, installed_app=None, installed_helper=None, published_installer=None, require_current_source=False):
    manifest = json.loads(workspace_path(root, manifest_path).read_text(encoding="utf-8"))
    if manifest.get("schemaVersion") != 1 or not manifest.get("completedAt"):
        raise ValueError("A completed build manifest is required")
    source = manifest.get("source")
    validate_source_identity(source)
    current_source_matches = source == source_identity(root)
    if require_current_source and not current_source_matches:
        raise ValueError("Current source does not match this build; previous installer retained")
    supplied = {"app": installed_app, "helper": installed_helper, "installer": published_installer}
    for name in ("app", "helper", "installer"):
        expected = manifest["artifacts"][name]
        if (not isinstance(expected, dict) or set(expected) != {"path", "bytes", "sha256"}
                or not isinstance(expected["path"], str) or Path(expected["path"]).is_absolute()
                or "\\" in expected["path"] or any(part in ("", ".", "..") for part in expected["path"].split("/"))
                or not isinstance(expected["bytes"], int) or expected["bytes"] < 2
                or not isinstance(expected["sha256"], str) or re.fullmatch(r"[0-9a-f]{64}", expected["sha256"]) is None):
            raise ValueError(f"Built {name} identity is malformed")
        # Explicit installation/distribution paths can replace the build-host
        # originals for a later check on another QA machine. Publication passes
        # none of these overrides and still requires every original artifact.
        if supplied[name] is None and executable(root, root / expected["path"]) != expected:
            raise ValueError(f"Built {name} does not match the manifest")
    if published_installer is not None:
        published = workspace_path(root, published_installer)
        if (sha256_file(published) != manifest["artifacts"]["installer"]["sha256"]
                or published.stat().st_size != manifest["artifacts"]["installer"]["bytes"]):
            raise ValueError("Published installer does not match this build")
    installed = {}
    for name, path in (("app", installed_app), ("helper", installed_helper)):
        if path is None:
            continue
        actual = sha256_file(path)
        if actual != manifest["artifacts"][name]["sha256"] or Path(path).stat().st_size != manifest["artifacts"][name]["bytes"]:
            raise ValueError(f"Installed {name} does not match this build")
        installed[name] = {"path": str(Path(path).absolute()), "sha256": actual}
    return {"schemaVersion": 1, "verifiedAt": utc_now(), "buildManifestSha256": sha256_file(manifest_path), "sourceCommit": source["commit"], "currentSourceMatches": current_source_matches, "publishedInstallerVerified": published_installer is not None, "installed": installed}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    start = sub.add_parser("begin")
    start.add_argument("--output", type=Path, required=True)
    complete = sub.add_parser("finish")
    for name in ("snapshot", "app", "helper", "installer", "output"):
        complete.add_argument(f"--{name}", type=Path, required=True)
    check = sub.add_parser("verify")
    check.add_argument("--manifest", type=Path, required=True)
    check.add_argument("--installed-app", type=Path)
    check.add_argument("--installed-helper", type=Path)
    check.add_argument("--published-installer", type=Path)
    check.add_argument("--require-current-source", action="store_true")
    check.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "begin":
            begin(ROOT, args.output)
        elif args.command == "finish":
            finish(ROOT, args.snapshot, args.app, args.helper, args.installer, args.output)
        else:
            result = verify(ROOT, args.manifest, args.installed_app, args.installed_helper, args.published_installer, args.require_current_source)
            if args.output:
                save_new(ROOT, args.output, result)
            print(json.dumps(result))
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Build evidence verification failed: {error}\n")


if __name__ == "__main__":
    main()
