"""Resume MSIX updates by submission ID; never modify listings/pricing or log SAS URLs."""
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

TERMINAL = {"Published", "Canceled", "CommitFailed", "PublishFailed", "PreProcessingFailed", "CertificationFailed", "ReleaseFailed", "error"}


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
    def __init__(self):
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
        return http(f"https://manage.devcenter.microsoft.com/v1.0/my/applications/{self.app}{suffix}",
                    json.dumps(data).encode() if data is not None else None, method,
                    {"Authorization": "Bearer " + self.token, "Content-Type": "application/json"})

    def upload(self, url, package):
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
    record = state(release)
    if record.get("storeSubmissionId"):
        submission_id = record["storeSubmissionId"]
        result = store.call(f"/submissions/{submission_id}/status")
        if result["status"] != "PendingCommit":
            record["store"] = result["status"]
            github.save_state(release, record)
            print(f"Store release {release['id']}: {result['status']}")
            return
    if not TAG.fullmatch(release["tag_name"]) or release.get("draft"):
        raise ValueError("Store updates require a published stable release")
    # Only download inert build outputs. Never execute release/PR artifacts.
    subprocess.run(["gh", "release", "download", release["tag_name"], "--repo", github.repository,
                    "--pattern", "release-manifest.json", "--dir", str(directory)], check=True)
    manifest = json.loads((directory / "release-manifest.json").read_text())
    receipt = next(item for item in manifest["artifacts"] if item["flavor"] == "store")
    if manifest["sourceSha"] != record["sourceSha"] or manifest["tag"] != release["tag_name"]:
        raise ValueError("Store release source/tag differs")
    subprocess.run(["gh", "run", "download", str(int(record["buildRunId"])), "--repo", github.repository,
                    "--name", "release-store", "--dir", str(directory)], check=True)
    name = receipt["binary"]
    if Path(name).name != name or not name.endswith(".msix"):
        raise ValueError("invalid Store package path")
    package = directory / name
    if package.is_symlink() or digest(package) != receipt["sha256"]:
        raise ValueError("Store package hash differs")
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


def main():
    github, store = GitHub(), Store()
    retry = os.environ.get("MON_STORE_RELEASE_ID", "")
    failures = []
    for release in github.releases():
        record = state(release)
        if not record or release.get("draft") or record.get("status") != "published":
            continue
        if retry and str(release["id"]) != str(int(retry)):
            continue
        if not retry and (record.get("store") in TERMINAL or record.get("store") == "initial_setup_required"):
            continue
        try:
            with tempfile.TemporaryDirectory(prefix="mon-store-") as temporary:
                resume(github, store, release, Path(temporary))
        except (ValueError, KeyError, OSError, subprocess.SubprocessError):
            record = state(github.release(release["id"]))
            record["store"] = "error"
            github.save_state(github.release(release["id"]), record)
            failures.append(str(release["id"]))
    if failures:
        raise ValueError("Store updates failed for release IDs " + ", ".join(failures) + "; credentials/response redacted")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError):
        raise SystemExit("Store operation failed; inspect release state and Partner Center (credentials redacted)")
