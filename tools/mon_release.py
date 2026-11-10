"""GitHub draft release controller. Only explicit opt-in drafts are actionable."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib
import urllib.error
from urllib.parse import quote
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
OPT_IN = "- [x] 自動公開を開始"
STATE = re.compile(r"\n?<!-- mon-release:([A-Za-z0-9+/=]+) -->")
TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def clean_body(body):
    return STATE.sub("", body or "").rstrip()


def state(release):
    matches = STATE.findall(release.get("body") or "")
    if len(matches) > 1:
        raise ValueError("multiple release state records")
    return json.loads(base64.b64decode(matches[0], validate=True)) if matches else None


def fingerprint(release):
    fields = {key: release.get(key) for key in ("id", "tag_name", "target_commitish", "name", "prerelease")}
    fields["body"] = clean_body(release.get("body"))
    return hashlib.sha256(json.dumps(fields, ensure_ascii=False, sort_keys=True).encode()).hexdigest()


def opted_in(release):
    # Exact whole line, never evaluate release notes as shell or template code.
    return OPT_IN in clean_body(release.get("body")).splitlines()


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
    if not release.get("draft") or release.get("prerelease") or not opted_in(release):
        raise ValueError("release must be an explicitly opted-in stable draft")
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


def discover(api):
    for release in api.releases():
        if not release.get("draft") or not opted_in(release) or state(release) is not None:
            continue
        # A draft can only be claimed by the trusted default-branch controller.
        author = quote(release["author"]["login"], safe="")
        permission = api.call(f"/collaborators/{author}/permission")["permission"]
        if permission not in ("admin", "maintain", "write"):
            raise ValueError("release author cannot publish")
        commit = candidate(api, release)
        record = {"schemaVersion": 1, "sourceSha": commit, "status": "claimed", "runId": os.environ["GITHUB_RUN_ID"]}
        release = api.save_state(release, record, target_commitish=commit)
        record["fingerprint"] = fingerprint(release)
        api.save_state(release, record)
        # workflow_dispatch is an explicit exception to GITHUB_TOKEN recursion
        # protection. It runs the workflow definition on main, not draft code.
        try:
            api.call("/actions/workflows/release-build.yml/dispatches", {
                "ref": "main", "inputs": {"release_id": str(release["id"]), "source_sha": commit,
                                           "dry_run": "false" if os.environ.get("RELEASE_PUBLISH_ENABLED") == "true" else "true"}})
        except Exception:
            record["status"] = "dispatch_failed"
            api.save_state(api.release(release["id"]), record)
            raise
        print(f"Dispatched draft release {release['id']} at {commit}")


def preflight(api, release_id, commit):
    if os.environ.get("GITHUB_REF") != "refs/heads/main":
        raise ValueError("release workflow must run from main")
    release = api.release(release_id)
    record = state(release)
    if (not record or record.get("sourceSha") != commit or record.get("fingerprint") != fingerprint(release)
            or record.get("status") not in ("claimed", "failed", "dispatch_failed")):
        raise ValueError("draft claim or fingerprint differs")
    if candidate(api, release) != commit:
        raise ValueError("draft tag moved after claim")
    return release


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


def publish(api, release_id, commit, directory):
    release = preflight(api, release_id, commit)
    manifest = json.loads((directory / "release-manifest.json").read_text())
    if manifest.get("sourceSha") != commit or manifest.get("tag") != release["tag_name"]:
        raise ValueError("release manifest source/tag differs")
    expected = manifest["publicFiles"]
    files = {p.name: p for p in directory.iterdir() if p.is_file()}
    if set(files) != {*expected, "release-manifest.json", "SHA256SUMS", "provenance.json"}:
        raise ValueError("release directory contains missing or unexpected assets")
    for name, digest in expected.items():
        if hashlib.sha256(files[name].read_bytes()).hexdigest() != digest:
            raise ValueError("public artifact hash differs")
    checksums = {**expected, "release-manifest.json": hashlib.sha256(files["release-manifest.json"].read_bytes()).hexdigest()}
    wanted = "".join(f"{value}  {name}\n" for name, value in sorted(checksums.items()))
    if files["SHA256SUMS"].read_text() != wanted:
        raise ValueError("public checksum manifest differs")
    if release.get("assets"):
        raise ValueError("draft already has assets; remove partial assets before retry")
    for name, file in sorted(files.items()):
        api.call(f"/releases/{release_id}/assets?name={quote(name, safe='')}", binary=file.read_bytes())
    # Re-fetch after uploads to detect user edits during the build/upload.
    release = preflight(api, release_id, commit)
    uploaded = {item["name"]: item for item in release.get("assets", [])}
    if set(uploaded) != set(files):
        raise ValueError("draft asset set changed before publication")
    for name, file in files.items():
        expected_digest = "sha256:" + hashlib.sha256(file.read_bytes()).hexdigest()
        if uploaded[name].get("digest") != expected_digest or uploaded[name].get("size") != file.stat().st_size:
            raise ValueError("uploaded artifact digest/size differs")
    record = state(release)
    record.update(status="published", buildRunId=os.environ["GITHUB_RUN_ID"], store="pending" if os.environ.get("STORE_PUBLISH_ENABLED") == "true" else "initial_setup_required")
    notes = clean_body(release.get("body")) + "\n\nWindows EXEは未署名です。実機確認・ライセンス審査の未確認項目は配布証跡を参照してください。"
    release["body"] = notes
    api.save_state(release, record, draft=False, make_latest="true")
    print(f"Published release {release_id}")


def failure(api, release_id):
    release = api.release(release_id)
    record = state(release)
    if release.get("draft") and record and record.get("status") != "published":
        record.update(status="failed", buildRunId=os.environ["GITHUB_RUN_ID"])
        release["body"] = clean_body(release.get("body")) + f"\n\n自動公開の失敗: https://github.com/{api.repository}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
        # A diagnostic link is our own edit; retain a retryable fingerprint.
        record["fingerprint"] = fingerprint(release)
        api.save_state(release, record)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("discover", "preflight", "publish", "failure", "checkout"))
    parser.add_argument("--release-id", type=int)
    parser.add_argument("--source-sha")
    parser.add_argument("--directory", type=Path)
    args = parser.parse_args()
    try:
        if args.mode == "checkout":
            checkout_source(args.source_sha)
            return 0
        api = GitHub()
        if args.mode == "discover":
            discover(api)
        elif args.mode == "failure":
            failure(api, args.release_id)
        elif args.mode == "preflight":
            preflight(api, args.release_id, args.source_sha)
        else:
            publish(api, args.release_id, args.source_sha, args.directory)
    except (ValueError, KeyError, OSError, TypeError, subprocess.SubprocessError) as error:
        print(f"Release operation failed: {error}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
