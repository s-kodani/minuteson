"""Attach saved main artifacts to stable drafts; publication stays in GitHub UI."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib
import tempfile
import urllib.error
from urllib.parse import quote
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
STATE = re.compile(r"\n?<!-- mon-release:([A-Za-z0-9+/=]+) -->")
TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def clean_body(body):
    return STATE.sub("", body or "").rstrip()


def state(release):
    matches = STATE.findall(release.get("body") or "")
    if len(matches) > 1:
        raise ValueError("multiple release state records")
    return json.loads(base64.b64decode(matches[0], validate=True)) if matches else None


class GitHub:
    def __init__(self, repository=None, token=None):
        self.repository = repository or os.environ["GITHUB_REPOSITORY"]
        self.token = token or os.environ.get("GH_TOKEN") or os.environ["GITHUB_TOKEN"]
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", self.repository):
            raise ValueError("invalid repository")

    def call(self, endpoint, data=None, method=None, binary=None):
        # GitHub's compare API requires a literal three-dot separator.
        main_comparison = re.fullmatch(r"/compare/[0-9a-f]{40}\.\.\.main", endpoint)
        if not endpoint.startswith("/") or (".." in endpoint and not main_comparison):
            raise ValueError("invalid GitHub endpoint")
        host = "https://uploads.github.com" if binary is not None else "https://api.github.com"
        payload = binary if binary is not None else (json.dumps(data).encode() if data is not None else None)
        request = urllib.request.Request(host + "/repos/" + self.repository + endpoint, payload, {
            "Authorization": "Bearer " + self.token, "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "MinutesOn-release",
            "Content-Type": "application/octet-stream" if binary is not None else "application/json",
        }, method=method)
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                raw = response.read(32 * 1024 * 1024 + 1)
                if len(raw) > 32 * 1024 * 1024:
                    raise ValueError("GitHub response exceeds metadata limit")
                return json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            # Response bodies and URLs may contain upload credentials; never echo them.
            raise ValueError(f"GitHub request failed ({error.code})") from None

    def releases(self):
        for page in range(1, 101):
            items = self.call(f"/releases?per_page=100&page={page}")
            yield from items
            if len(items) < 100:
                return
        raise ValueError("release pagination exceeds limit")

    def release(self, release_id):
        return self.call(f"/releases/{int(release_id)}")

    def save_state(self, release, record, **changes):
        body = clean_body(release.get("body"))
        encoded = base64.b64encode(json.dumps(record, sort_keys=True).encode()).decode()
        if release.get("draft"):
            # GitHub can replace an unpublished tag with an untagged placeholder on PATCH.
            changes["tag_name"] = release["tag_name"]
        return self.call(f"/releases/{release['id']}", {**changes, "body": body + f"\n\n<!-- mon-release:{encoded} -->"}, "PATCH")

    def file(self, name, commit):
        result = self.call(f"/contents/{name}?ref={quote(commit, safe='')}")
        return base64.b64decode(result["content"]).decode()


def validate_versions(tag, read):
    if not isinstance(tag, str) or not TAG.fullmatch(tag):
        raise ValueError("stable vX.Y.Z release tag required")
    versions = [json.loads(read("app/package.json"))["version"],
                json.loads(read("app/package-lock.json"))["packages"][""]["version"],
                json.loads(read("app/src-tauri/tauri.conf.json"))["version"],
                tomllib.loads(read("app/src-tauri/Cargo.toml"))["package"]["version"]]
    if any(version != tag[1:] for version in versions):
        raise ValueError("release tag differs from application versions")


def validate_draft(release):
    if not release.get("draft") or release.get("prerelease"):
        raise ValueError("release must be a stable draft")
    if not TAG.fullmatch(release.get("tag_name", "")):
        raise ValueError("invalid release tag")
    target = release.get("target_commitish", "")
    if target != "main" and not re.fullmatch(r"[0-9a-f]{40}", target):
        raise ValueError("release target must be main or a full commit SHA")


def candidate(api, release):
    validate_draft(release)
    # GitHub may create the tag only when publishing. Resolve an existing tag
    # when selected; otherwise use the target, and bind it before dispatch.
    refs = api.call("/git/matching-refs/tags/" + quote(release["tag_name"], safe=""))
    exists = any(ref["ref"] == "refs/tags/" + release["tag_name"] for ref in refs)
    reference = release["tag_name"] if exists else release["target_commitish"]
    commit = api.call("/commits/" + quote(reference, safe=""))["sha"]
    comparison = api.call(f"/compare/{commit}...main")
    if comparison["status"] not in ("ahead", "identical"):
        raise ValueError("release source is not contained in main")
    validate_versions(release["tag_name"], lambda name: api.file(name, commit))
    return commit


def assert_draft(api, release_id, commit, tag, *, resolve=False):
    release = api.release(release_id)
    validate_draft(release)
    if release["target_commitish"] != commit or release["tag_name"] != tag:
        raise ValueError("draft source or tag changed; saved artifacts will not be substituted")
    if resolve and candidate(api, release) != commit:
        raise ValueError("draft tag moved; saved artifacts will not be substituted")
    return release


def asset_metadata(files):
    return {name: {"digest": "sha256:" + hashlib.sha256(file.read_bytes()).hexdigest(),
                   "size": file.stat().st_size} for name, file in files.items()}


def matching_assets(release, expected):
    existing = {}
    for asset in release.get("assets", []):
        name = asset["name"]
        if name in existing or name not in expected:
            raise ValueError("draft contains duplicate or unexpected assets; nothing will be overwritten")
        if any(asset.get(key) != value for key, value in expected[name].items()):
            raise ValueError("draft asset conflicts with saved main artifacts; nothing will be overwritten")
        existing[name] = asset
    return set(existing) == set(expected)


def public_files(directory, commit, tag):
    manifest_file = directory / "release-manifest.json"
    manifest = json.loads(manifest_file.read_text())
    if manifest.get("sourceSha") != commit or manifest.get("tag") != tag or manifest.get("schemaVersion") != 2:
        raise ValueError("main candidate source/tag differs")
    expected = manifest["publicFiles"]
    files = {p.name: p for p in directory.iterdir()}
    if (any(not p.is_file() or p.is_symlink() for p in files.values()) or
            set(files) != {*expected, "release-manifest.json", "SHA256SUMS", "provenance.json"}):
        raise ValueError("candidate contains missing or unexpected assets")
    for name, digest in expected.items():
        if hashlib.sha256(files[name].read_bytes()).hexdigest() != digest:
            raise ValueError("public artifact hash differs")
    checksums = {**expected, "release-manifest.json": hashlib.sha256(manifest_file.read_bytes()).hexdigest()}
    wanted = "".join(f"{value}  {name}\n" for name, value in sorted(checksums.items()))
    if files["SHA256SUMS"].read_text() != wanted:
        raise ValueError("public checksum manifest differs")
    return files, manifest


def attach(api, release_id, commit, saved, directory):
    from mon_store_provenance import verify_manifest
    release = api.release(release_id)
    tag = release["tag_name"]
    files, manifest = public_files(directory, commit, tag)
    proof = verify_manifest(api, release, directory / "release-manifest.json", draft=True, expected_run=saved["runId"])
    if proof is None:
        raise ValueError("main candidate is not complete yet")
    expected = asset_metadata(files)
    release = assert_draft(api, release_id, commit, tag, resolve=True)
    matching_assets(release, expected)
    for name, file in sorted(files.items()):
        release = assert_draft(api, release_id, commit, tag)
        matching_assets(release, expected)
        if any(asset["name"] == name for asset in release.get("assets", [])):
            continue
        api.call(f"/releases/{release_id}/assets?name={quote(name, safe='')}", binary=file.read_bytes())
    release = assert_draft(api, release_id, commit, tag, resolve=True)
    if not matching_assets(release, expected):
        raise ValueError("draft attachment is incomplete")
    record = state(release) or {}
    record.update(schemaVersion=2, sourceSha=commit, status="attached", buildRunId=saved["runId"],
                  candidateArtifactId=saved["artifactId"], buildRunAttempt=manifest["producer"]["runAttempt"],
                  assets=expected, store=record.get("store", "awaiting_approval"))
    api.save_state(release, record)  # Never set draft=False or create/push a tag.
    print(f"Attached main candidate to draft {release_id}; Publish release remains manual")


def discover(api):
    from mon_main_build import find_candidate, PREFIX
    if os.environ.get("GITHUB_REF") != "refs/heads/main":
        raise ValueError("draft controller must run from main")
    failures = []
    for release in api.releases():
        if not release.get("draft") or release.get("prerelease") or not TAG.fullmatch(release.get("tag_name", "")):
            continue
        try:
            author = quote(release["author"]["login"], safe="")
            if api.call(f"/collaborators/{author}/permission")["permission"] not in ("admin", "maintain", "write"):
                raise ValueError("release author cannot publish")
            commit = candidate(api, release)
            record = state(release) or {}
            if record.get("sourceSha") not in (None, commit):
                raise ValueError("draft source changed; create a new draft for a different candidate")
            if record.get("status") == "attached" and matching_assets(release, record["assets"]):
                continue
            saved = find_candidate(api, commit)
            if not saved:
                raise ValueError("no completed main candidate; wait for main build or check artifact retention")
            if record.get("candidateArtifactId") not in (None, saved["artifactId"]):
                raise ValueError("draft candidate changed; refusing artifact substitution")
            record.update(schemaVersion=2, sourceSha=commit, candidateArtifactId=saved["artifactId"], status="attaching")
            api.save_state(release, record, target_commitish=commit)
            with tempfile.TemporaryDirectory(prefix="mon-candidate-") as temporary:
                subprocess.run(["gh", "run", "download", str(saved["runId"]), "--repo", api.repository,
                                "--name", PREFIX + commit, "--dir", temporary], check=True)
                attach(api, release["id"], commit, saved, Path(temporary))
        except (ValueError, KeyError, OSError, TypeError, subprocess.SubprocessError) as error:
            print(f"Draft {release['id']}: {error}")
            failures.append(str(release["id"]))
    if failures:
        raise ValueError("draft attachment failed for release IDs " + ", ".join(failures))


def checkout_source(commit):
    """Run from trusted main before loading any code from the admitted source."""
    if os.environ.get("GITHUB_REF") != "refs/heads/main" or not re.fullmatch(r"[0-9a-f]{40}", commit or ""):
        raise ValueError("main workflow and full source SHA required")
    status = subprocess.run(["git", "status", "--porcelain"], cwd=ROOT,
                            capture_output=True, text=True, check=True)
    if status.stdout.strip():
        raise ValueError("source checkout requires a clean working tree")
    contained = subprocess.run(["git", "merge-base", "--is-ancestor", commit, "origin/main"],
                               cwd=ROOT, capture_output=True, check=False)
    if contained.returncode:
        raise ValueError("release source is not contained in fetched main")
    subprocess.run(["git", "checkout", "--detach", commit], cwd=ROOT, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("discover", "checkout"))
    parser.add_argument("--source-sha")
    args = parser.parse_args()
    try:
        if args.mode == "checkout":
            checkout_source(args.source_sha)
            return 0
        api = GitHub()
        discover(api)
    except (ValueError, KeyError, OSError, TypeError, subprocess.SubprocessError) as error:
        print(f"Release operation failed: {error}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
