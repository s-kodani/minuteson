"""Collect installed dependency notices offline; this is NOT a license clearance.

The selected target graph conservatively includes build dependencies, not
development or unreachable/platform-only packages. Package-provided texts are copied verbatim;
missing texts and source/license obligations remain explicit review work.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib

REPOSITORY = Path(__file__).resolve().parents[1]
NOTICE_NAME = re.compile(r"^(licen[sc]e|copying|notice|copyright)(?:$|[._-])", re.I)
NOTICE_DIRECTORY = re.compile(r"^(licen[sc]es?|notices?)$", re.I)
SOURCE_SUFFIXES = {".rs", ".py", ".c", ".cpp", ".cc", ".h", ".hpp", ".m", ".mm",
                   ".swift", ".js", ".ts", ".go", ".rb", ".java", ".sh"}
MAX_TEXT_BYTES = 1024 * 1024
MAX_TOTAL_BYTES = 16 * 1024 * 1024
OUTPUTS = ("THIRD_PARTY_NOTICES.txt", "third-party-inventory.json")
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def cargo_packages(metadata):
    graph = metadata["resolve"]
    nodes = {node["id"]: node for node in graph["nodes"]}
    root = graph["root"]
    if not root or root not in nodes:
        raise ValueError("Cargo metadata must have a resolved root package")
    reached, pending = set(), [root]
    while pending:
        package = pending.pop()
        if package in reached:
            continue
        reached.add(package)
        for dependency in nodes[package]["deps"]:
            if any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    packages = {package["id"]: package for package in metadata["packages"]}
    return [packages[key] for key in sorted(reached - {root})]


def notice_files(base, declared=None):
    """Never follow package symlinks, including an explicit license-file."""
    if base.is_symlink() or not base.is_dir() or base.resolve() != base:
        raise ValueError("dependency directory is missing or linked")
    result = set()
    if declared:
        path = Path(declared)
        path = path if path.is_absolute() else base / path
        if not path.resolve().is_relative_to(base):
            raise ValueError("declared license file escapes its dependency")
        if path.is_file():
            result.add(path)
    for parent, directories, files in os.walk(base, followlinks=False):
        directories[:] = sorted(name for name in directories
                                if name not in (".git", "target", "node_modules")
                                and not (Path(parent) / name).is_symlink())
        for name in files:
            relative_parent = Path(parent).relative_to(base)
            in_notices = any(NOTICE_DIRECTORY.fullmatch(part) for part in relative_parent.parts)
            if (NOTICE_NAME.match(name) or in_notices) and Path(name).suffix.lower() not in SOURCE_SUFFIXES:
                result.add(Path(parent) / name)
    texts = []
    for path in sorted(result):
        if path.is_symlink() or path.resolve() != path or not path.is_file():
            raise ValueError("notice file is linked or not a regular file")
        if path.stat().st_size > MAX_TEXT_BYTES:
            raise ValueError("dependency notice exceeds the size limit")
        data = path.read_bytes()
        data.decode("utf8")  # Do not silently omit or replace invalid text.
        if data.strip():
            texts.append((str(path.relative_to(base)), data))
    return texts


def supplemental_files(repository, package, index, locked):
    matched = [entry for entry in index["packages"]
               if (entry["name"], entry["version"]) == (package["name"], package["version"])]
    if len(matched) > 1:
        raise ValueError("duplicate supplemental package")
    if not matched:
        return []
    entry = matched[0]
    base = Path(package["manifest_path"]).parent
    vcs_path = base / ".cargo_vcs_info.json"
    if vcs_path.is_symlink() or vcs_path.resolve() != vcs_path or vcs_path.stat().st_size > 65536:
        raise ValueError("crate VCS metadata is linked or oversized")
    vcs = json.loads(vcs_path.read_text(encoding="utf8"))
    if (package.get("source") != REGISTRY or package.get("license") != entry["declaredLicense"]
            or locked[(package["name"], package["version"], REGISTRY)]["checksum"] != entry["crateChecksum"]
            or vcs.get("git", {}).get("sha1") != entry["vcsCommit"]
            or vcs.get("path_in_vcs") != entry["vcsPath"]):
        raise ValueError("supplemental notice differs from the exact crate/commit/license")
    directory = repository / "third-party/notices"
    files = {record["path"]: record for record in index["files"]}
    result = []
    for name in entry["files"]:
        record = files[name]
        path = directory / name
        if (Path(name).is_absolute() or ".." in Path(name).parts or path.is_symlink()
                or path.resolve() != path or not path.is_file() or path.stat().st_size > MAX_TEXT_BYTES):
            raise ValueError("supplemental notice path is invalid, linked or oversized")
        data = path.read_bytes()
        data.decode("utf8")
        if (not data.strip() or sha(data) != record["sha256"] or len(data) != record["bytes"]
                or record["kind"] not in ("license_text", "upstream_license_reference", "license_template")
                or not record["sourceUrl"].startswith("https://")):
            raise ValueError("supplemental notice content/provenance changed")
        result.append((record, data))
    return result


def collect(repository, metadata, lock, target="aarch64-apple-darwin"):
    if target not in ("aarch64-apple-darwin", "x86_64-pc-windows-msvc"):
        raise ValueError("unsupported notice target")
    if lock.get("lockfileVersion") != 3:
        raise ValueError("npm lockfile version 3 is required")
    index_path = repository / "third-party/notices/index.json"
    if index_path.is_symlink() or index_path.resolve() != index_path or index_path.stat().st_size > MAX_TEXT_BYTES:
        raise ValueError("supplemental notice index is linked or oversized")
    index_bytes = index_path.read_bytes()
    index = json.loads(index_bytes)
    if index["schemaVersion"] != 1:
        raise ValueError("unsupported supplemental notice index")
    cargo_lock = tomllib.loads((repository / "app/src-tauri/Cargo.lock").read_text(encoding="utf8"))
    locked = {(entry["name"], entry["version"], entry.get("source")): entry for entry in cargo_lock.get("package", [])}
    dependencies = []
    for package in cargo_packages(metadata):
        base = Path(package["manifest_path"]).parent
        dependencies.append(("cargo", package["name"], package["version"],
                             package.get("license"), package.get("repository"),
                             notice_files(base, package.get("license_file")),
                             supplemental_files(repository, package, index, locked),
                             ({"url": f"https://static.crates.io/crates/{package['name']}/{package['name']}-{package['version']}.crate",
                               "sha256": locked[(package["name"], package["version"], REGISTRY)]["checksum"],
                               "state": "source_reference_not_download_verified"}
                              if package.get("source") == REGISTRY else None)))
    for location, package in sorted(lock["packages"].items()):
        if not location or package.get("dev") is True:
            continue
        parts = Path(location).parts
        if (len(parts) < 2 or parts[0] != "node_modules" or ".." in parts
                or Path(location).is_absolute() or package.get("link")):
            raise ValueError("npm dependency location is invalid or linked")
        base = repository / "app" / location
        installed = json.loads((base / "package.json").read_text(encoding="utf8"))
        expected_name = "/".join(parts[len(parts) - 1 - int(parts[-2].startswith("@")):])
        if installed["version"] != package["version"] or installed["name"] != expected_name:
            raise ValueError("installed npm name/version differs from the lockfile")
        dependencies.append(("npm", installed["name"], package["version"],
                             package.get("license"), None, notice_files(base), [], None))
    dependencies.sort(key=lambda item: item[:3])
    inventory = {"schemaVersion": 1, "target": target,
                 "scope": "resolved Cargo normal/build dependencies and npm production dependencies",
                 "licenseReview": "not_verified", "distributionReady": False,
                 "excluded": ["MON-owned code", "Rust standard library/toolchain", ("Apple SDK/system frameworks" if target == "aarch64-apple-darwin" else "Microsoft Windows SDK/system libraries"),
                              "separately obtained AI engines/models/runtime libraries"],
                 "lockHashes": {name: sha((repository / relative).read_bytes()) for name, relative in
                                (("cargo", "app/src-tauri/Cargo.lock"), ("npm", "app/package-lock.json"))},
                 "supplementManifestSha256": sha(index_bytes),
                 "packages": [], "missingNoticeTexts": [], "referenceOnlyPackages": []}
    chunks = [b"MON third-party dependency notices\n\n"
              b"Collected package-provided texts, NOT a license clearance.\n"
              b"See third-party-inventory.json for scope, missing texts and exact versions.\n"
              b"Source availability, additional notices and model/engine licenses need review.\n\n"]
    total = 0
    for ecosystem, name, version, license_id, repository_url, texts, supplements, source in dependencies:
        key = f"{ecosystem}:{name}@{version}"
        entry = {"ecosystem": ecosystem, "name": name, "version": version,
                 "declaredLicense": license_id, "repository": repository_url, "noticeFiles": [],
                 "supplementalFiles": [record for record, _ in supplements], "sourceArchive": source}
        if not texts and not any(record["kind"] == "license_text" for record, _ in supplements):
            inventory["missingNoticeTexts"].append(key)
            if supplements:
                inventory["referenceOnlyPackages"].append(key)
        chunks.append(f"{'=' * 72}\n{key}\nDeclared license: {license_id or 'not supplied'}\n\n".encode())
        for relative, data in texts:
            total += len(data)
            if total > MAX_TOTAL_BYTES:
                raise ValueError("dependency notices exceed the total size limit")
            entry["noticeFiles"].append({"path": relative, "bytes": len(data), "sha256": sha(data)})
            chunks.extend([f"--- {relative} ---\n".encode(), data, b"\n\n"])
        for record, data in supplements:
            total += len(data)
            if total > MAX_TOTAL_BYTES:
                raise ValueError("dependency notices exceed the total size limit")
            if record["kind"] == "license_template":
                chunks.append(b"Standard license template only; NOT a package-specific copyright notice.\n")
            chunks.extend([f"--- Supplemental {record['kind']}: {record['path']} ---\nSource: {record['sourceUrl']}\n".encode(),
                           data, b"\n\n"])
        if source and license_id and "MPL-2.0" in license_id:
            chunks.append(f"MPL source reference (retrieval not verified): {source['url']}\n"
                          f"Expected source archive SHA-256: {source['sha256']}\n\n".encode())
        inventory["packages"].append(entry)
    notices = b"".join(chunks)
    inventory["noticesSha256"] = sha(notices)
    inventory["noticesBytes"] = len(notices)
    return notices, inventory


def write_outputs(output, notices, inventory):
    if not output.is_absolute() or output.is_symlink() or output.resolve() != output:
        raise ValueError("generated notice directory must be an absolute non-linked path")
    output.mkdir(parents=True, exist_ok=True)
    for name in OUTPUTS:
        path = output / name
        if path.is_symlink() or (path.exists() and not path.is_file()):
            raise ValueError("generated notice output is linked or not a file")
    for name, data in zip(OUTPUTS, (notices, (json.dumps(inventory, ensure_ascii=False, indent=2) + "\n").encode())):
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=output, prefix=".notice-", delete=False) as stream:
                temporary = Path(stream.name)
                if os.name != "nt":
                    os.fchmod(stream.fileno(), 0o644)
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            # Windows cannot replace or delete a still-open temporary file.
            os.replace(temporary, output / name)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--target", choices=("aarch64-apple-darwin", "x86_64-pc-windows-msvc"),
                        default="aarch64-apple-darwin")
    args = parser.parse_args()
    output = args.output_dir or (REPOSITORY / ".cache" /
                                 ("windows-third-party-notices" if args.target == "x86_64-pc-windows-msvc"
                                  else "third-party-notices"))
    try:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--offline", "--locked", "--format-version", "1",
             "--filter-platform", args.target], cwd=REPOSITORY / "app/src-tauri", timeout=120))
        lock = json.loads((REPOSITORY / "app/package-lock.json").read_text(encoding="utf8"))
        notices, inventory = collect(REPOSITORY, metadata, lock, args.target)
        write_outputs(output, notices, inventory)
        print(json.dumps({"packages": len(inventory["packages"]), "bytes": len(notices),
                          "missingNoticeTexts": inventory["missingNoticeTexts"], "licenseReview": "not_verified"}))
        return 0
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        parser.exit(1, f"Notice collection failed: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
