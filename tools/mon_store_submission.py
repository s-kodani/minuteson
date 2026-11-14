"""Resume MSIX updates by submission ID; never modify listings/pricing or log SAS URLs."""
import argparse
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from urllib.parse import urlencode, urlsplit
import urllib.error
import urllib.request
import zipfile

from mon_release import GitHub, TAG, state
from mon_release_artifacts import digest
from mon_store_provenance import verify_manifest

TERMINAL = {"Published", "Canceled", "CommitFailed", "PublishFailed", "PreProcessingFailed", "CertificationFailed", "ReleaseFailed"}


def http(url, data=None, method=None, headers=None):
    try:
        request = urllib.request.Request(url, data, headers or {}, method=method)
        with urllib.request.urlopen(request, timeout=120) as response:
            raw = response.read(32 * 1024 * 1024 + 1)
        if len(raw) > 32 * 1024 * 1024:
            raise ValueError("Store response exceeds limit")
        return json.loads(raw) if raw else None
    except urllib.error.HTTPError as error:
        raise ValueError(f"Store request failed ({error.code}); credentials/response redacted") from None


class Store:
    def __init__(self, *, read_only=False):
        self.read_only = read_only
        tenant = os.environ["STORE_TENANT_ID"]
        self.app = os.environ["STORE_APPLICATION_ID"]
        if not re.fullmatch(r"[0-9a-fA-F-]{36}", tenant) or not re.fullmatch(r"[A-Za-z0-9]+", self.app):
            raise ValueError("invalid Store tenant/application ID")
        result = http(f"https://login.microsoftonline.com/{tenant}/oauth2/token", urlencode({
            "grant_type": "client_credentials", "client_id": os.environ["STORE_CLIENT_ID"],
            "client_secret": os.environ["STORE_CLIENT_SECRET"], "resource": "https://manage.devcenter.microsoft.com",
        }).encode(), headers={"Content-Type": "application/x-www-form-urlencoded"})
        self.token = result["access_token"]

    def call(self, suffix="", data=None, method=None):
        if not re.fullmatch(r"(?:/submissions(?:/[A-Za-z0-9]+(?:/(?:status|commit))?)?)?", suffix):
            raise ValueError("invalid submission API path")
        if self.read_only and (method not in (None, "GET") or data is not None or not suffix.endswith("/status")):
            raise ValueError("Store polling only permits GET submission status")
        return http(f"https://manage.devcenter.microsoft.com/v1.0/my/applications/{self.app}{suffix}",
                    json.dumps(data).encode() if data is not None else None, method,
                    {"Authorization": "Bearer " + self.token, "Content-Type": "application/json"})

    def upload(self, url, package):
        if self.read_only:
            raise ValueError("Store polling cannot upload packages")
        parsed = urlsplit(url)
        if parsed.scheme != "https" or not parsed.hostname or not parsed.hostname.endswith(".blob.core.windows.net") or parsed.username or parsed.password:
            raise ValueError("Store returned an invalid Azure upload URL")
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
            output.write(package, package.name)
        http(url, archive.getvalue(), "PUT", {"x-ms-blob-type": "BlockBlob", "Content-Type": "application/zip"})


def update_submission(submission, package_name):
    result = dict(submission)
    # Keep the copied listing/age rating/pricing data; replace only app packages.
    packages, replacement = [], None
    for item in result.get("applicationPackages", []):
        if item["fileName"] == package_name:
            # After PUT succeeds, a failed upload/commit leaves this package in
            # the stored submission. Reuse its ID, metadata and upload status.
            if replacement is not None or item.get("fileStatus") not in ("PendingUpload", "Uploaded"):
                raise ValueError("Store target package is duplicated or marked for deletion")
            replacement = dict(item)
        else:
            packages.append(dict(item, fileStatus="PendingDelete"))
    if replacement is None:
        replacement = {"fileName": package_name, "fileStatus": "PendingUpload", "minimumDirectXVersion": "None", "minimumSystemRam": "None"}
    result["applicationPackages"] = [*packages, replacement]
    result["targetPublishMode"] = "Immediate"
    return result


def resume(github, store, release, directory):
    record = state(release) or {}
    record["status"] = "published"
    if record.get("storeSubmissionId"):
        submission_id = record["storeSubmissionId"]
        result = store.call(f"/submissions/{submission_id}/status")
        if result["status"] != "PendingCommit":
            record["store"] = result["status"]
            github.save_state(release, record)
            print(f"Store release {release['id']}: {result['status']}")
            return
    if not isinstance(release.get("tag_name"), str) or not TAG.fullmatch(release["tag_name"]) or release.get("draft") or release.get("prerelease"):
        raise ValueError("Store updates require a published stable release")
    # Only download inert build outputs. Never execute release/PR artifacts.
    subprocess.run(["gh", "release", "download", release["tag_name"], "--repo", github.repository,
                    "--pattern", "release-manifest.json", "--dir", str(directory)], check=True)
    proof = verify_manifest(github, release, directory / "release-manifest.json")
    if proof is None:
        print(f"Store release {release['id']}: release publication pending")
        return
    receipt, build_run_id = proof
    record["sourceSha"] = receipt["sourceSha"]
    package_directory = directory / "package"
    package_directory.mkdir()
    subprocess.run(["gh", "run", "download", str(build_run_id), "--repo", github.repository,
                    "--name", "release-store", "--dir", str(package_directory)], check=True)
    name = receipt["binary"]
    package = package_directory / name
    if not package.is_file() or package.is_symlink() or package.stat().st_size != receipt["bytes"] or digest(package) != receipt["sha256"]:
        raise ValueError("Store package size/hash differs")
    if not record.get("storeSubmissionId"):
        application = store.call()
        if application.get("pendingApplicationSubmission"):
            raise ValueError("Partner Center already has a pending submission; reconcile it before retry")
        if not application.get("lastPublishedApplicationSubmission"):
            raise ValueError("first Store publication must be completed in Partner Center")
        submission = store.call("/submissions", method="POST")
        record.update(storeSubmissionId=submission["id"], storePackageSha256=receipt["sha256"], store="PendingCommit")
        # Persist ID immediately. If this write fails, the next attempt sees the
        # existing pending submission and stops instead of creating another.
        release = github.save_state(release, record)
    else:
        if record.get("storePackageSha256") != receipt["sha256"]:
            raise ValueError("cannot replace the package of an existing submission")
        submission = store.call(f"/submissions/{record['storeSubmissionId']}")
    submission_id = record["storeSubmissionId"]
    store.call(f"/submissions/{submission_id}", update_submission(submission, name), "PUT")
    store.upload(submission["fileUploadUrl"], package)
    store.call(f"/submissions/{submission_id}/commit", method="POST")
    record["store"] = "CommitStarted"
    github.save_state(github.release(release["id"]), record)
    print(f"Store release {release['id']}: submission committed; publication pending")


def poll(github, store, release):
    """Observe an existing ID, including PendingCommit, without resuming writes."""
    record = state(release) or {}
    submission_id = record.get("storeSubmissionId")
    if not submission_id:
        return
    result = store.call(f"/submissions/{submission_id}/status")
    # Merge the current state so a concurrent approved submission keeps its ID
    # and package binding. No build download, creation, PUT, upload, or commit.
    current = github.release(release["id"])
    latest = state(current) or {}
    if latest.get("storeSubmissionId") != submission_id:
        raise ValueError("Store submission changed during status polling")
    latest["store"] = result["status"]
    github.save_state(current, latest)
    print(f"Store release {release['id']}: {result['status']}")


def main(mode="submit"):
    github = GitHub()
    selected = os.environ.get("MON_STORE_RELEASE_ID", "")
    if selected and not re.fullmatch(r"[1-9][0-9]*", selected):
        raise ValueError("positive release ID required")
    if mode == "request":
        release = github.release(int(selected))
        if release.get("draft") or release.get("prerelease") or not TAG.fullmatch(release.get("tag_name", "")):
            raise ValueError("Store approval requires a published stable release")
        github.call("/actions/workflows/store-submission.yml/dispatches", {
            "ref": "main", "inputs": {"release_id": selected, "operation": "submit"}})
        return
    if mode == "submit" and not selected:
        raise ValueError("Store submission requires an explicit release ID and Environment approval")
    if mode == "submit" and os.environ.get("GITHUB_REF") != "refs/heads/main":
        raise ValueError("Store submission must run from main")
    store = Store(read_only=mode == "poll")
    failures = []
    releases = [github.release(int(selected))] if selected else github.releases()
    for release in releases:
        record = state(release)
        if release.get("draft") or release.get("prerelease"):
            continue
        if mode == "poll" and (not record or not record.get("storeSubmissionId") or
                               (not selected and record.get("store") in TERMINAL)):
            continue
        try:
            if mode == "poll":
                poll(github, store, release)
            else:
                with tempfile.TemporaryDirectory(prefix="mon-store-") as temporary:
                    resume(github, store, release, Path(temporary))
        except (ValueError, KeyError, OSError, subprocess.SubprocessError):
            if mode == "submit":
                current = github.release(release["id"])
                record = state(current) or {}
                record["store"] = "error"
                github.save_state(current, record)
            failures.append(str(release["id"]))
    if failures:
        raise ValueError("Store updates failed for release IDs " + ", ".join(failures) + "; credentials/response redacted")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("submit", "poll", "request"))
    try:
        main(parser.parse_args().mode)
    except (ValueError, KeyError, OSError):
        raise SystemExit("Store operation failed; inspect release state and Partner Center (credentials redacted)")
