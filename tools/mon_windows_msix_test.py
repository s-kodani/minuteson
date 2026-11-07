"""Prepare/verify a payload-identical, locally signed copy of a Store MSIX.

No certificate changes, signing, installation, app control, or user-data IO.
Windows signing is performed separately by prepare-windows-store-test.ps1.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import posixpath
import re
import shutil
import subprocess
import sys
from urllib.parse import quote, urlsplit, urlunsplit
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PACKAGING_FILES = {"AppxBlockMap.xml", "[Content_Types].xml"}
SIGNING_FILES = {"AppxSignature.p7x", "AppxMetadata/CodeIntegrity.cat"}
METADATA_TAGS = {"AXBM": "AppxBlockMap.xml", "AXCT": "[Content_Types].xml", "AXCI": "AppxMetadata/CodeIntegrity.cat"}
VALIDATION_GUIDE = "検証手順.md"
GUIDE_SAMPLE_DIRECTORY = r"C:\path\to\windows-store-test\今回の検証フォルダー"
GUIDE_SAMPLE_THUMBPRINT = "今回案内された40文字の証明書拇印"
sys.dont_write_bytecode = True


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def save_new(path, value):
    with Path(path).open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, ensure_ascii=True, indent=2)
        stream.write("\n")


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verified_build(workspace, manifest_path, expected_commit):
    workspace = Path(workspace).resolve(strict=True)
    if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", expected_commit):
        raise ValueError("ExpectedCommit must be the full pinned Git commit")
    def git(*args):
        return subprocess.check_output(["git", "-C", str(workspace), *args]).decode().strip()
    if git("rev-parse", "HEAD") != expected_commit:
        raise ValueError("Build workspace is not at ExpectedCommit")
    if git("status", "--porcelain=v1", "--untracked-files=all"):
        raise ValueError("Build workspace must be clean; the source guard cannot be bypassed")
    spec = importlib.util.spec_from_file_location("test_copy_frozen_msix", workspace / "tools/mon_windows_msix.py")
    msix = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(msix)
    manifest_path = msix.regular(workspace, Path(manifest_path).resolve(strict=True))
    manifest = read_json(manifest_path)
    artifact = msix.regular(workspace, workspace / manifest["package"]["path"])
    msix.verify(workspace, manifest_path, artifact, require_current_source=True)
    return msix, workspace, manifest_path, manifest, artifact


def output_path(path):
    path = Path(path).absolute()
    if ".." in path.parts:
        raise ValueError("Local test output cannot traverse parent directories")
    if not path.is_relative_to(ROOT / "output"):
        raise ValueError("Local test outputs must stay inside the repository output directory")
    for part in (path, *path.parents):
        if part == ROOT:
            break
        if part.is_symlink() or (hasattr(part, "is_junction") and part.is_junction()):
            raise ValueError("Local test output cannot follow a link")
    return path


def prepare(args):
    msix, workspace, manifest_path, build, artifact = verified_build(args.build_workspace, args.build_manifest, args.expected_commit)
    original = msix.regular(ROOT if args.store_package else workspace, Path(args.store_package).absolute() if args.store_package else artifact)
    if sha(original) != build["package"]["sha256"]:
        raise ValueError("Unsigned Store original differs from frozen build evidence")
    msix.verify_payload(original, build["identity"], build["files"])
    if not re.fullmatch(r"[A-Za-z0-9 ._-]+", build["identity"]["displayName"]):
        raise ValueError("Unsafe package display name")
    destination = output_path(args.output)
    destination.mkdir(parents=True, exist_ok=False)
    name = f"{build['identity']['displayName']}_{build['identity']['version']}_x64-local-test.msix"
    copy = destination / name
    shutil.copyfile(original, copy)
    if sha(copy) != build["package"]["sha256"] or sha(original) != build["package"]["sha256"]:
        raise ValueError("Unsigned copy differs from original; nothing may be signed")
    with zipfile.ZipFile(copy) as archive:
        blockmap_sha = hashlib.sha256(archive.read("AppxBlockMap.xml")).hexdigest()
    plan = {
        "schemaVersion": 1, "purpose": "payload-identical-local-store-test", "createdAt": msix.evidence.utc_now(),
        "buildWorkspace": str(workspace), "buildManifest": str(manifest_path), "expectedCommit": args.expected_commit,
        "buildManifestSha256": sha(manifest_path), "originalPackage": str(original),
        "originalSha256": build["package"]["sha256"], "unsignedCopySha256": sha(copy),
        "testPackage": str(copy), "identity": build["identity"], "files": build["files"],
        "originalBlockMapSha256": blockmap_sha, "signtool": str(msix.sdk_tool("signtool.exe")),
    }
    save_new(destination / "test-plan.json", plan)
    print(f"Verified exact unsigned copy: {copy}; payload/version were not changed")


def verify_signed_payload(package, files, metadata_hashes, original_blockmap_sha):
    """Pure ZIP/hash verification, separately paired with Windows CMS checks."""
    expected = {entry["path"]: entry for entry in files}
    if len(expected) != len(files):
        raise ValueError("Duplicate payload evidence")
    with zipfile.ZipFile(package) as archive:
        names = [entry.filename for entry in archive.infolist()]
        if len(names) != len({name.casefold() for name in names}) or set(names) != set(expected) | PACKAGING_FILES | SIGNING_FILES:
            raise ValueError("Unexpected signed package inventory")
        for name, entry in expected.items():
            if archive.getinfo(name).file_size != entry["bytes"]:
                raise ValueError(f"Signed payload size mismatch: {name}")
            with archive.open(name) as source:
                if hashlib.file_digest(source, "sha256").hexdigest() != entry["sha256"]:
                    raise ValueError(f"Signed payload hash mismatch: {name}")
        if not archive.read("AppxSignature.p7x").startswith(b"PKCX"):
            raise ValueError("Unexpected MSIX signature header")
        catalog = archive.read("AppxMetadata/CodeIntegrity.cat")
        if not catalog or catalog[0] != 0x30:
            raise ValueError("Unexpected signing-generated code integrity catalog")
        if set(metadata_hashes) != set(METADATA_TAGS):
            raise ValueError("Signed metadata hash evidence is incomplete")
        for tag, name in METADATA_TAGS.items():
            digest = hashlib.sha256(archive.read(name)).hexdigest()
            if digest != metadata_hashes[tag]:
                raise ValueError(f"CMS-signed metadata differs: {name}")
            if tag == "AXBM" and digest != original_blockmap_sha:
                raise ValueError("Signing changed the original block map")
    return len(files)


def render_validation_guide(msix, workspace, plan, result):
    """Make the canonical Markdown guide usable with this exact signed copy."""
    guide_path = workspace / "docs/guides/testing-windows-msix.md"
    try:
        source = msix.regular(workspace, guide_path).read_text(encoding="utf-8-sig")
    except (OSError, ValueError) as error:
        raise ValueError(f"Canonical MSIX validation guide is unavailable: {guide_path}") from error
    header = re.match(r"\A---\n.*?\n---\n", source, re.DOTALL)
    body = source[header.end():].lstrip() if header else ""
    if (not body.startswith("# ") or GUIDE_SAMPLE_DIRECTORY not in body
            or GUIDE_SAMPLE_THUMBPRINT not in body or "```powershell" not in body):
        raise ValueError("Canonical MSIX validation guide has invalid Markdown or missing substitution fields")
    test_directory = str(Path(plan["testPackage"]).parent).replace("'", "''")
    body = body.replace(GUIDE_SAMPLE_DIRECTORY, test_directory)
    body = body.replace(GUIDE_SAMPLE_THUMBPRINT, result["certificate"]["thumbprint"])

    def pin_document_link(match):
        target = urlsplit(match[2])
        if target.scheme or target.netloc or not target.path:
            return match[0]
        path = posixpath.normpath(posixpath.join("docs/guides", target.path))
        if path == ".." or path.startswith("../"):
            raise ValueError("Canonical MSIX guide link traverses outside the repository")
        pinned = urlunsplit(("https", "github.com", "/s-kodani/minuteson/blob/"
                            + result["buildCommit"] + "/" + quote(path, safe="/"), target.query, target.fragment))
        return f"[{match[1]}]({pinned})"

    body = re.sub(r"(?<!!)\[([^\]]+)\]\(([^)]+)\)", pin_document_link, body)
    fields = [
        ("ビルドコミット", result["buildCommit"]),
        ("MSIXバージョン", result["testVersion"]),
        ("CPU", plan["identity"]["architecture"]),
        ("署名済みMSIX", Path(result["signedTestPackage"]).name),
        ("署名済みMSIXのSHA256", result["signedTestSha256"]),
        ("公開証明書のSHA256", result["certificate"]["publicCertificateSha256"]),
        ("証明書拇印", result["certificate"]["thumbprint"]),
        ("証明書期限（UTC）", result["certificate"]["notAfter"]),
        ("検証フォルダー", str(Path(plan["testPackage"]).parent)),
        ("未署名原本", result["originalPackage"]),
        ("未署名原本のSHA256", result["originalSha256"]),
        ("ビルド証跡", result["buildManifest"]),
    ]
    def code(value):
        value = str(value).replace("\r", " ").replace("\n", " ")
        fence = "`" * (max((len(run) for run in re.findall(r"`+", value)), default=0) + 1)
        return f"{fence} {value} {fence}"
    metadata = "\n".join(f"- {label}: {code(value)}" for label, value in fields)
    title, remainder = body.split("\n", 1)
    return (f"{title}\n\n## 今回の成果物\n\n{metadata}\n\n"
            f"署名の暗号学的検証と原本からの全{result['payloadFiles']}ペイロード・版数の一致を確認済みです。\n"
            "信頼登録・インストール・アプリ停止・実機機能確認・後片付けは未実施です。\n"
            "署名鍵は作成者のCurrentUser\\Myに保持し、秘密鍵は配布しません。\n\n"
            "このMarkdownはビルド元の検証ガイドから自動生成しました。別端末では各コード例の"
            "`$testDirectory`をコピー先へ変更してください。削除コマンドは手順のみで自動実行しません。\n\n"
            + remainder.lstrip())


def finish(args):
    plan_path = output_path(args.plan)
    plan = read_json(plan_path)
    msix, workspace, manifest_path, build, _ = verified_build(plan["buildWorkspace"], plan["buildManifest"], plan["expectedCommit"])
    if sha(manifest_path) != plan["buildManifestSha256"] or build["identity"] != plan["identity"] or build["files"] != plan["files"]:
        raise ValueError("Frozen build evidence changed")
    original = msix.regular(ROOT if Path(plan["originalPackage"]).is_relative_to(ROOT) else workspace, Path(plan["originalPackage"]))
    if sha(original) != plan["originalSha256"]:
        raise ValueError("Unsigned Store original changed")
    report_path = output_path(args.signature_report)
    if report_path.parent != plan_path.parent:
        raise ValueError("Signature report must be alongside the prepared copy")
    report = read_json(report_path)
    certificate = report["certificate"]
    if report.get("signatureCryptoVerified") is not True or report.get("windowsTrustVerified") is not False:
        raise ValueError("Explicit cryptographic verification without an unproven Windows-trust claim is required")
    if certificate["subject"] != plan["identity"]["publisher"] or not re.fullmatch(r"[0-9A-F]{40}", certificate["thumbprint"]):
        raise ValueError("Certificate does not match the expected Publisher")
    if (certificate.get("persistentKeyRetained") is not True or certificate.get("nonExportable") is not True
            or certificate.get("privateKeyExported") is not False):
        raise ValueError("Persistent nonexportable test key evidence is required")
    public_certificate = output_path(certificate["publicCertificate"])
    if public_certificate.parent != plan_path.parent or sha(public_certificate) != certificate["publicCertificateSha256"]:
        raise ValueError("Exported public certificate changed")
    package = output_path(plan["testPackage"])
    if package.parent != plan_path.parent:
        raise ValueError("Signed copy must remain in its preparation folder")
    count = verify_signed_payload(package, plan["files"], report["metadataHashes"], plan["originalBlockMapSha256"])
    result = {
        "schemaVersion": 1, "status": "prepared-for-manual-local-update", "preparedAt": msix.evidence.utc_now(),
        "purpose": plan["purpose"], "buildWorkspace": str(workspace), "buildManifest": str(manifest_path),
        "buildCommit": plan["expectedCommit"], "originalPackage": str(original), "originalSha256": plan["originalSha256"],
        "originalUnchanged": True, "signedTestPackage": str(package), "signedTestSha256": sha(package),
        "originalVersion": plan["identity"]["version"], "testVersion": plan["identity"]["version"],
        "payloadFiles": count, "unchangedPayloadFiles": count, "payloadChanges": [], "manifestVersionIdentical": True,
        "certificate": certificate, "signatureCryptoVerified": True, "windowsTrustVerified": False,
        "persistentSigningKeyRetained": True, "osDisplayVerification": "pending", "storeTransitionVerified": False,
        "certificateTrustImported": False, "packageInstalled": False, "applicationStopped": False,
    }
    guide = render_validation_guide(msix, workspace, plan, result)
    guide_path = plan_path.parent / VALIDATION_GUIDE
    with guide_path.open("x", encoding="utf-8", newline="\n") as stream:
        stream.write(guide)
    save_new(plan_path.parent / "test-manifest.json", result)
    print(f"Verified signed copy: {count} byte-identical payload files; version {result['testVersion']}; no install or trust changes")
    print(f"Validation guide: {guide_path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare_command = commands.add_parser("prepare")
    for name in ("build-workspace", "build-manifest", "expected-commit", "output"):
        prepare_command.add_argument("--" + name, required=True)
    prepare_command.add_argument("--store-package")
    verify_command = commands.add_parser("finish")
    verify_command.add_argument("--plan", required=True)
    verify_command.add_argument("--signature-report", required=True)
    args = parser.parse_args()
    try:
        prepare(args) if args.command == "prepare" else finish(args)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        parser.exit(1, f"Store local-test preparation failed: {error}\n")


if __name__ == "__main__":
    main()
