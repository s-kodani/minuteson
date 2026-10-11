"""Explicit macOS distribution build; never discover credentials or launch MON.

The Tauri preparation is deliberately ad-hoc and receives no notarization
credentials. Only our copied output is signed inside-out, with distinct helper
entitlements. Uploads require an explicit --notary-profile argument.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid

REPOSITORY = Path(__file__).resolve().parents[1]
TEAM = re.compile(r"[A-Z0-9]{10}\Z")
NOTARY_ENV = ("APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH",
              "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_PROVIDER_SHORT_NAME")
POLICIES = {"main.plist": {}, "capture.plist": {"com.apple.security.device.audio-input": True},
            "embedding.plist": {"com.apple.security.cs.disable-library-validation": True}}


def credentials(identity, team, profile):
    if not TEAM.fullmatch(team):
        raise ValueError("expected Team ID must be 10 uppercase letters/digits")
    if (not identity.startswith("Developer ID Application: ")
            or not identity.endswith(f" ({team})")
            or any(ord(c) < 32 or ord(c) == 127 for c in identity)):
        raise ValueError("APPLE_SIGNING_IDENTITY must name a Developer ID Application identity for the expected Team ID")
    if profile is not None and (not profile.strip() or profile.startswith("-") or len(profile) > 128
                                or any(ord(c) < 32 or ord(c) == 127 for c in profile)):
        raise ValueError("notary profile name is invalid")


def build_environment(environment, cache):
    result = dict(environment)
    # Signing/notarization happens below, never implicitly inside Tauri.
    for key in (*NOTARY_ENV, "APPLE_SIGNING_IDENTITY"):
        result.pop(key, None)
    result.update(CARGO_TARGET_DIR=str(cache), CARGO_INCREMENTAL="0")
    return result


def requirement(team):
    if not TEAM.fullmatch(team):
        raise ValueError("invalid Team ID")
    # codesign interprets an unprefixed argument as a requirement FILE. The
    # leading '=' selects inline source, not a path or our displayed metadata.
    return ('=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists '
            'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists '
            f'and certificate leaf[subject.OU] = "{team}"')


def command(argv, *, cwd=None, env=None, timeout=1800):
    """Own the process group; interrupt/timeout must not leave build children."""
    process = subprocess.Popen(argv, cwd=cwd, env=env, start_new_session=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
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
    if len(stdout) + len(stderr) > 4 * 1024 * 1024:
        raise ValueError(f"{Path(argv[0]).name} output exceeded the diagnostic limit")
    if process.returncode:
        # Do not print credential-bearing environments or entire notary logs.
        raise ValueError(f"{Path(argv[0]).name} failed (exit {process.returncode}); retained build artifacts are not distribution-ready")
    return stdout, stderr


def private_path(path, *, directory=False):
    if not path.is_absolute() or path.is_symlink():
        raise ValueError("artifact must be an absolute, non-linked path")
    if not (path.is_dir() if directory else path.is_file()):
        raise ValueError("expected artifact is missing")
    if path.resolve() != path:
        raise ValueError("artifact parent contains a link")
    return path


def read_plist(path):
    private_path(path)
    if path.stat().st_size > 65536:
        raise ValueError("bundle plist exceeds the metadata limit")
    with path.open("rb") as stream:
        value = plistlib.load(stream)
    if not isinstance(value, dict):
        raise ValueError("bundle plist is not a dictionary")
    return value


def layout(app, version):
    private_path(app, directory=True)
    main = read_plist(app / "Contents/Info.plist")
    def purpose(info, key):
        value = info.get(key)
        return isinstance(value, str) and bool(value.strip()) and len(value.encode("utf8")) <= 4096
    if (main.get("CFBundleIdentifier") != "jp.mon.desktop"
            or main.get("CFBundleShortVersionString") != version
            or main.get("CFBundleVersion") != version
            or main.get("LSMinimumSystemVersion") != "15.0"
            or main.get("CFBundleExecutable") != "mon-desktop"
            or not purpose(main, "NSMicrophoneUsageDescription")
            or not purpose(main, "NSAudioCaptureUsageDescription")):
        raise ValueError("production app identifier/version/minimum OS/executable does not match")
    main_executable = private_path(app / "Contents/MacOS/mon-desktop")
    capture = app / "Contents/Resources/MONCaptureProbe.app"
    private_path(capture, directory=True)
    info = read_plist(capture / "Contents/Info.plist")
    if (info.get("CFBundleIdentifier") != "app.mon.capture.probe"
            or info.get("CFBundleExecutable") != "mon-macos-capture"
            or not purpose(info, "NSMicrophoneUsageDescription")
            or not purpose(info, "NSAudioCaptureUsageDescription")):
        raise ValueError("capture helper identity or privacy declarations are missing")
    capture_executable = private_path(capture / "Contents/MacOS/mon-macos-capture")
    private_path(capture / "Contents/Resources/PrivacyInfo.xcprivacy")
    embedding = private_path(app / "Contents/Resources/mon-speaker-embedding")
    for executable in (main_executable, capture_executable, embedding):
        if executable.stat().st_mode & 0o111 == 0:
            raise ValueError("distribution executable has no execute permission")
        architectures, _ = command(["/usr/bin/lipo", "-archs", str(executable)], timeout=30)
        if architectures.strip() != b"arm64":
            raise ValueError("distribution executable is not Apple Silicon arm64 only")
    return capture, embedding


def verify_signature(path, team, *, runtime, entitlements=None):
    expected_policy = None
    if entitlements is not None:
        expected_policy = POLICIES.get(entitlements.name)
        if expected_policy is None or read_plist(entitlements) != expected_policy:
            raise ValueError("component entitlement policy changed or is unsupported")
    command(["/usr/bin/codesign", "--verify", "--deep", "--strict", "-R",
             requirement(team), str(path)], timeout=120)
    stdout, stderr = command(["/usr/bin/codesign", "--display", "--verbose=4", str(path)], timeout=30)
    details = (stdout + stderr).decode("utf8", errors="strict")
    if not re.search(rf"^TeamIdentifier={re.escape(team)}$", details, re.M):
        raise ValueError("signed Team ID does not match")
    if not re.search(r"^Authority=Developer ID Application: .+", details, re.M):
        raise ValueError("signature is not Developer ID Application")
    if not re.search(r"^Timestamp=.+", details, re.M):
        raise ValueError("secure signature timestamp is missing")
    flags = re.search(r"\bflags=0x([0-9a-fA-F]+)", details)
    if runtime and (not flags or int(flags[1], 16) & 0x10000 == 0):
        raise ValueError("hardened runtime is missing")
    if entitlements is not None:
        stdout, _ = command(["/usr/bin/codesign", "--display", "--entitlements", "-", "--xml", str(path)], timeout=30)
        actual = plistlib.loads(stdout) if stdout.strip() else {}
        if actual != expected_policy:
            raise ValueError("signed entitlements do not match the explicit component policy")


def sign(path, identity, team, entitlements=None):
    argv = ["/usr/bin/codesign", "--force", "--sign", identity, "--timestamp"]
    if entitlements is not None:
        argv += ["--options", "runtime", "--entitlements", str(entitlements)]
    command([*argv, str(path)], timeout=180)
    verify_signature(path, team, runtime=entitlements is not None, entitlements=entitlements)


def notarize(path, profile):
    stdout, _ = command(["/usr/bin/xcrun", "notarytool", "submit", str(path),
                         "--keychain-profile", profile, "--wait", "--output-format", "json"], timeout=1800)
    result = json.loads(stdout)
    if not isinstance(result, dict):
        raise ValueError("notary service response is not a result object")
    try:
        request_id = str(uuid.UUID(result.get("id", "")))
    except (ValueError, AttributeError, TypeError):
        raise ValueError("notary service response has no canonical request ID") from None
    if result.get("status") != "Accepted" or result["id"] != request_id:
        raise ValueError("notary service did not accept the artifact; no release success")
    return result["id"]


def digest(path):
    private_path(path)
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def expected_notices():
    """Recollect offline from the current locked inputs; never write or approve."""
    spec = importlib.util.spec_from_file_location("mon_release_notices", REPOSITORY / "tools/mon_third_party_notices.py")
    collector = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(collector)
    stdout, _ = command(["cargo", "metadata", "--offline", "--locked", "--format-version", "1",
                         "--filter-platform", "aarch64-apple-darwin"],
                        cwd=REPOSITORY / "app/src-tauri", timeout=120)
    lock = json.loads(private_path(REPOSITORY / "app/package-lock.json").read_text(encoding="utf8"))
    return collector.collect(REPOSITORY, json.loads(stdout), lock)


def verify_notices(app):
    """Verify collection/copy evidence, never elevate it to license approval."""
    inventory_path = private_path(app / "Contents/Resources/third-party-inventory.json")
    text_path = private_path(app / "Contents/Resources/THIRD_PARTY_NOTICES.txt")
    if inventory_path.stat().st_size > 2 * 1024 ** 2 or text_path.stat().st_size > 17 * 1024 ** 2:
        raise ValueError("dependency notice bundle exceeds the size limit")
    inventory = json.loads(inventory_path.read_text(encoding="utf8"))
    expected_locks = {name: digest(REPOSITORY / relative) for name, relative in
                      (("cargo", "app/src-tauri/Cargo.lock"), ("npm", "app/package-lock.json"))}
    if (not isinstance(inventory, dict) or inventory.get("schemaVersion") != 1
            or inventory.get("target") != "aarch64-apple-darwin"
            or inventory.get("lockHashes") != expected_locks
            or inventory.get("supplementManifestSha256") != digest(REPOSITORY / "third-party/notices/index.json")
            or inventory.get("licenseReview") != "not_verified"
            or inventory.get("distributionReady") is not False
            or not isinstance(inventory.get("packages"), list) or not inventory["packages"]
            or not isinstance(inventory.get("missingNoticeTexts"), list)
            or inventory.get("noticesBytes") != text_path.stat().st_size
            or inventory.get("noticesSha256") != digest(text_path)):
        raise ValueError("dependency notices are missing, stale or changed; no license approval")
    expected_text, expected_inventory = expected_notices()
    if (text_path.read_bytes() != expected_text
            or json.dumps(inventory, sort_keys=True) != json.dumps(expected_inventory, sort_keys=True)):
        raise ValueError("dependency notices differ from current offline collection; no license approval")
    return {"state": "collected_not_reviewed", "packages": len(inventory["packages"]),
            "missingNoticeTexts": inventory["missingNoticeTexts"],
            "supplementManifestSha256": inventory["supplementManifestSha256"],
            "noticesSha256": inventory["noticesSha256"], "inventorySha256": digest(inventory_path)}


def verify_asr_runtime(app):
    """Sealed build-time resource proof, NOT signed runtime/onboarding proof.

    The payload CLI is still ad-hoc and inside a data TAR. Outer app signing or
    notarization must not be reported as Developer ID signing of that CLI.
    """
    spec = importlib.util.spec_from_file_location("mon_release_asr_runtime", REPOSITORY / "tools/mon_whisper_runtime.py")
    runtime = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runtime)
    index = runtime.verify_package(private_path(app / "Contents/Resources/asr-runtime", directory=True))
    return {"state": "packaged_onboarding_unverified", "engineVersion": index["engineVersion"],
            "sourceCommit": index["sourceCommit"], "archiveSha256": index["archive"]["sha256"],
            "archiveBytes": index["archive"]["bytes"], "runtimeSigning": index["runtimeSigning"],
            "developerIdSigning": "not_verified", "standaloneNotarization": "not_verified",
            "nativeOnboarding": "not_verified", "licenseReview": "not_verified"}


def build(output, identity, team, profile, environment):
    # All explicit prerequisites precede mkdir, compilation, signing or upload.
    credentials(identity, team, profile)
    if sys.platform != "darwin":
        raise ValueError("distribution build supports macOS only")
    if not output.is_absolute() or output.exists() or output.is_symlink():
        raise ValueError("output must be a new absolute directory; existing contents are never overwritten")
    private_path(output.parent, directory=True)
    if shutil.disk_usage(output.parent).free < 4 * 1024 ** 3:
        raise ValueError("at least 4GiB free is required for the temporary build and copies")
    config = json.loads((REPOSITORY / "app/src-tauri/tauri.conf.json").read_text())
    version = config["version"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("distribution version must be a numeric release version")
    policy = REPOSITORY / "app/src-tauri/release-entitlements"
    for name, expected in POLICIES.items():
        if read_plist(policy / name) != expected:
            raise ValueError("component entitlement policy changed; refusing before build")
    output.mkdir(mode=0o700)
    cache = Path(tempfile.mkdtemp(prefix="mon-distribution-build-", dir=output))
    cache_identity = cache.stat()
    timings = {}
    try:
        started = time.monotonic()
        stdout, stderr = command([str(REPOSITORY / "app/node_modules/.bin/tauri"), "build", "--config",
                 "src-tauri/tauri.release.conf.json"], cwd=REPOSITORY / "app",
                env=build_environment(environment, cache))
        timings["buildSeconds"] = round(time.monotonic() - started, 3)
        with (output / "preparation-build.log").open("xb") as stream:
            stream.write(stdout + stderr)
        built = cache / "release/bundle/macos/mon.app"
        capture, embedding = layout(built, version)
        app = output / "mon.app"
        command(["/usr/bin/ditto", str(built), str(app)], timeout=120)
        capture, embedding = layout(app, version)
        verify_notices(app)
        verify_asr_runtime(app)
        started = time.monotonic()
        sign(capture, identity, team, policy / "capture.plist")
        sign(embedding, identity, team, policy / "embedding.plist")
        sign(app, identity, team, policy / "main.plist")
        timings["signAndVerifySeconds"] = round(time.monotonic() - started, 3)
        requests = []
        if profile is not None:
            # Staple the nested app before constructing the final DMG. Never
            # rebuild or mutate a DMG after obtaining that DMG's ticket.
            archive = output / "mon-notary.zip"
            command(["/usr/bin/ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(archive)], timeout=120)
            requests.append(notarize(archive, profile))
            command(["/usr/bin/xcrun", "stapler", "staple", str(app)], timeout=180)
            command(["/usr/bin/xcrun", "stapler", "validate", str(app)], timeout=180)
        image_source = output / "image-source"
        image_source.mkdir(mode=0o700)
        command(["/usr/bin/ditto", str(app), str(image_source / "mon.app")], timeout=120)
        (image_source / "Applications").symlink_to("/Applications")
        dmg = output / f"mon_{version}_aarch64.dmg"
        command(["/usr/bin/hdiutil", "create", "-volname", "MON", "-srcfolder", str(image_source),
                 "-format", "UDZO", "-nospotlight", str(dmg)], timeout=300)
        command(["/usr/bin/hdiutil", "verify", str(dmg)], timeout=120)
        sign(dmg, identity, team)
        if profile is not None:
            requests.append(notarize(dmg, profile))
            command(["/usr/bin/xcrun", "stapler", "staple", str(dmg)], timeout=180)
            command(["/usr/bin/xcrun", "stapler", "validate", str(dmg)], timeout=180)
        # Reverify the signed components after stapling. A valid signature or
        # accepted ticket does not prove GUI, Gatekeeper, model or update QA.
        for path, policy_file in ((capture, "capture.plist"), (embedding, "embedding.plist"), (app, "main.plist")):
            verify_signature(path, team, runtime=True, entitlements=policy / policy_file)
        verify_signature(dmg, team, runtime=False)
        dependency_notices = verify_notices(app)
        asr_runtime = verify_asr_runtime(app)
        # Only this newly-created cache is reclaimed, after all owned commands
        # have returned and final artifacts are verified. Failures preserve it.
        now = cache.lstat()
        if cache.is_symlink() or (now.st_dev, now.st_ino) != (cache_identity.st_dev, cache_identity.st_ino):
            raise ValueError("build cache identity changed; preserve instead of cleanup")
        shutil.rmtree(cache)
        result = {"schemaVersion": 1, "version": version, "teamId": team,
                  "dmg": dmg.name, "dmgSha256": digest(dmg), "dmgBytes": dmg.stat().st_size,
                  "notarization": "tickets_validated" if profile else "not_requested",
                  "notaryRequestIds": requests, "runtimeQA": "not_verified", "gatekeeper": "not_verified",
                  "licenseReview": "not_verified", "manualUpdate": "not_verified", "distributionReady": False,
                  "dependencyNotices": dependency_notices,
                  "asrRuntime": asr_runtime,
                  "timings": timings, "buildCache": str(cache), "buildCacheRetained": False}
        # Create-new-only; no credentials or secret material in the report.
        with (output / "build-result.json").open("x") as stream:
            json.dump(result, stream, ensure_ascii=False, indent=2)
        return result
    except BaseException:
        print(f"Build interrupted/failed. Retained output: {output}; build cache retained: {cache.exists()}", file=sys.stderr)
        raise


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--team-id", required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--sign-only", action="store_true", help="no uploads; not notarized")
    mode.add_argument("--notary-profile", help="explicitly authorize two uploads using an existing notarytool keychain profile")
    args = parser.parse_args(argv)
    try:
        result = build(args.output_dir, os.environ.get("APPLE_SIGNING_IDENTITY", ""),
                       args.team_id, args.notary_profile, os.environ)
        print(json.dumps(result, ensure_ascii=False))
        return 0
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
