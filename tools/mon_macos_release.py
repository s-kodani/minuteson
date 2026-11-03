"""Explicit macOS distribution build; never discover credentials or launch MinutesOn.

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
import posixpath
import stat
import zipfile

REPOSITORY = Path(__file__).resolve().parents[1]
TEAM = re.compile(r"[A-Z0-9]{10}\Z")
NOTARY_ENV = ("APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH",
              "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_PROVIDER_SHORT_NAME")
POLICIES = {"main.plist": {"com.apple.security.device.audio-input": True}, "capture.plist": {"com.apple.security.device.audio-input": True},
            "embedding.plist": {"com.apple.security.cs.disable-library-validation": True}}


def release_configuration(repository=None):
    """Read the same base, platform and explicit overrides used by Tauri."""
    root = REPOSITORY if repository is None else repository

    def merge(base, patch):
        # Tauri uses JSON Merge Patch: objects merge, arrays replace, null removes.
        if not isinstance(patch, dict):
            return patch
        result = dict(base) if isinstance(base, dict) else {}
        for key, value in patch.items():
            if value is None:
                result.pop(key, None)
            else:
                result[key] = merge(result.get(key), value)
        return result

    config = {}
    for name in ("tauri.conf.json", "tauri.macos.conf.json", "tauri.release.conf.json"):
        value = json.loads((root / "app/src-tauri" / name).read_text(encoding="utf8"))
        if not isinstance(value, dict):
            raise ValueError("Tauri release configuration must be an object")
        config = merge(config, value)
    name = config.get("productName")
    if (not isinstance(name, str) or not name or name != name.strip()
            or len(name.encode("utf8")) > 128 or not name.isprintable()
            or name in (".", "..") or name.startswith("-") or name.endswith(".")
            or any(c in name for c in '/\\:<>"|?*')):
        raise ValueError("distribution productName must be a safe app and disk image name")
    version = config.get("version")
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("distribution version must be a numeric release version")
    return config


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


def layout(app, version, product_name):
    private_path(app, directory=True)
    main = read_plist(app / "Contents/Info.plist")
    def purpose(info, key):
        value = info.get(key)
        return isinstance(value, str) and bool(value.strip()) and len(value.encode("utf8")) <= 4096
    if (main.get("CFBundleIdentifier") != "jp.mon.desktop"
            or main.get("CFBundleName") != product_name
            or main.get("CFBundleDisplayName") != product_name
            or main.get("CFBundleShortVersionString") != version
            or main.get("CFBundleVersion") != version
            or main.get("LSMinimumSystemVersion") != "15.0"
            or main.get("CFBundleExecutable") != "mon-desktop"
            or not purpose(main, "NSMicrophoneUsageDescription")
            or not purpose(main, "NSAudioCaptureUsageDescription")):
        raise ValueError("production app name/identifier/version/minimum OS/executable does not match")
    main_executable = private_path(app / "Contents/MacOS/mon-desktop")
    capture = app / "Contents/Resources/MONCaptureProbe.app"
    private_path(capture, directory=True)
    info = read_plist(capture / "Contents/Info.plist")
    if (info.get("CFBundleIdentifier") != "app.mon.capture.probe"
            or info.get("CFBundleExecutable") != "mon-macos-capture"
            or info.get("CFBundleName") != "MinutesOn Capture Helper"
            or info.get("CFBundleDisplayName") != "MinutesOn Capture Helper"
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


def verify_notices(app, expected=None):
    """Verify collection/copy evidence, never elevate it to license approval."""
    inventory_path = private_path(app / "Contents/Resources/third-party-inventory.json")
    text_path = private_path(app / "Contents/Resources/THIRD_PARTY_NOTICES.txt")
    if inventory_path.stat().st_size > 2 * 1024 ** 2 or text_path.stat().st_size > 17 * 1024 ** 2:
        raise ValueError("dependency notice bundle exceeds the size limit")
    inventory = json.loads(inventory_path.read_text(encoding="utf8"))
    expected_locks = {name: digest(REPOSITORY / relative) for name, relative in
                      (("cargo", "Cargo.lock"), ("npm", "app/package-lock.json"))}
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
    if expected is None:
        expected_text, expected_inventory = expected_notices()
        if (text_path.read_bytes() != expected_text
                or json.dumps(inventory, sort_keys=True) != json.dumps(expected_inventory, sort_keys=True)):
            raise ValueError("dependency notices differ from current offline collection; no license approval")
    result = {"state": "collected_not_reviewed", "packages": len(inventory["packages"]),
              "missingNoticeTexts": inventory["missingNoticeTexts"],
              "supplementManifestSha256": inventory["supplementManifestSha256"],
              "noticesSha256": inventory["noticesSha256"], "inventorySha256": digest(inventory_path)}
    if expected is not None and result != expected:
        raise ValueError("dependency notices differ from the prepared collection receipt")
    return result


def asr_module():
    spec = importlib.util.spec_from_file_location("mon_release_asr_runtime", REPOSITORY / "tools/mon_whisper_runtime.py")
    runtime = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runtime)
    # One supervised command boundary, including Mach-O and signature checks.
    runtime.command = command
    return runtime


WORKER_NAMES = ("mon-whisper-worker", "mon-llama-worker", "mon-sherpa-worker", "mon-transducer-worker")

def verify_inference_runtime(app, team=None):
    root = private_path(app / "Contents/Resources/inference-runtime", directory=True)
    index_file = private_path(root / "runtime-index.json")
    if index_file.stat().st_size > 65536:
        raise ValueError("worker index exceeds the metadata limit")
    index = json.loads(index_file.read_text())
    if not isinstance(index, list) or len(index) != len(WORKER_NAMES):
        raise ValueError("worker index is incomplete")
    expected = dict(zip(("whisper", "llama", "sherpa", "asr-transducer"), WORKER_NAMES))
    if {item.get("engine") for item in index} != set(expected):
        raise ValueError("worker index engines differ")
    runtime = asr_module()
    for item in index:
        if (set(item) != {"protocol", "engine", "name", "bytes", "sha256"}
                or item["protocol"] != 1 or item["name"] != expected[item["engine"]]):
            raise ValueError("worker identity is invalid")
        binary = private_path(root / item["name"])
        if binary.stat().st_size != item["bytes"] or digest(binary) != item["sha256"]:
            raise ValueError("worker bytes differ from the fixed index")
        runtime.verify_binary(binary)
        if team is not None:
            policy = "embedding.plist" if item["engine"] in ("sherpa", "asr-transducer") else "main.plist"
            verify_signature(binary, team, runtime=True,
                             entitlements=REPOSITORY / "app/src-tauri/release-entitlements" / policy)
    for notice in ("whisper-MIT.txt", "llama-MIT.txt", "nlohmann-MIT.txt", "sherpa-Apache-2.0.txt", "build-info.json"):
        if not private_path(root / notice).stat().st_size:
            raise ValueError("worker provenance or license notice is missing")
    return index

def sign_inference_runtime(app, identity, team, policy):
    index = verify_inference_runtime(app)
    root = app / "Contents/Resources/inference-runtime"
    for item in index:
        binary = root / item["name"]
        sign(binary, identity, team, policy / ("embedding.plist" if item["engine"] in ("sherpa", "asr-transducer") else "main.plist"))
        item["bytes"], item["sha256"] = binary.stat().st_size, digest(binary)
    # Final signed bytes are the identities frozen into future jobs. This runs
    # only in the private release copy, before sealing the enclosing app.
    (root / "runtime-index.json").write_text(json.dumps(index, indent=2) + "\n")
    spec = importlib.util.spec_from_file_location("mon_release_asr_bundle", REPOSITORY / "tools/mon_asr_bundle.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.assemble(root)
    return verify_inference_runtime(app, team)


def sign_asr_runtime(app, identity, team, policy):
    """Reseal the CLI in our copied, not-yet-signed app; never change the cache."""
    credentials(identity, team, None)
    runtime = asr_module()
    directory = private_path(app / "Contents/Resources/asr-runtime", directory=True)
    with tempfile.TemporaryDirectory(prefix="mon-asr-sign-", dir=app.parent) as temporary:
        work = Path(temporary)
        extracted = work / "extracted"
        extracted.mkdir(mode=0o700)
        before = runtime.unpack_package(directory, extracted)
        if before["runtimeSigning"] != "ad_hoc_only":
            raise ValueError("release preparation must start from the original ad-hoc runtime")
        binary = extracted / "bin/whisper-cli"
        runtime.verify_binary(binary)
        # Static CPU/Metal with embedded shaders; use the normal app policy.
        sign(binary, identity, team, policy / "main.plist")
        runtime.verify_binary(binary)
        package = work / "package"
        package.mkdir(mode=0o700)
        after = runtime.package(binary, extracted / "notices/whisper-MIT.txt", package, signing_team=team)
        if after["files"][1:] != before["files"][1:]:
            raise ValueError("runtime signing changed the notice or pinned build provenance")
        # Only the two known resources in our private copy are replaced, before
        # signing its outer bundle. A failure cannot publish a success result.
        for name in ("whisper-runtime.tar", "runtime-index.json"):
            private_path(directory / name)
            os.replace(private_path(package / name), directory / name)
    return verify_asr_runtime(app, team)


def verify_asr_runtime(app, team=None):
    """Hash proof; with an explicit Team ID, also check the actual inner CLI.

    Neither an outer app ticket nor JSON metadata proves standalone
    notarization, Gatekeeper acceptance, native onboarding or license approval.
    """
    runtime = asr_module()
    index = runtime.verify_package(private_path(app / "Contents/Resources/asr-runtime", directory=True))
    signing = "not_verified"
    if team is not None:
        if (not TEAM.fullmatch(team) or index["runtimeSigning"] != "developer_id_signed"
                or index.get("signingTeamId") != team):
            raise ValueError("release runtime signing metadata does not match the expected Team ID")
        with tempfile.TemporaryDirectory(prefix="mon-asr-verify-", dir=app.parent) as temporary:
            extracted = Path(temporary)
            checked = runtime.unpack_package(app / "Contents/Resources/asr-runtime", extracted)
            if checked != index:
                raise ValueError("runtime package changed during signature verification")
            binary = extracted / "bin/whisper-cli"
            runtime.verify_binary(binary)
            verify_signature(binary, team, runtime=True,
                             entitlements=REPOSITORY / "app/src-tauri/release-entitlements/main.plist")
        signing = "verified"
    return {"state": "packaged_onboarding_unverified", "engineVersion": index["engineVersion"],
            "sourceCommit": index["sourceCommit"], "archiveSha256": index["archive"]["sha256"],
            "archiveBytes": index["archive"]["bytes"], "runtimeSigning": index["runtimeSigning"],
            "developerIdSigning": signing, "standaloneNotarization": "not_verified",
            "nativeOnboarding": "not_verified", "licenseReview": "not_verified"}


def prepared_receipt(prepared):
    """Check an inert handoff before loading credentials or invoking ditto."""
    private_path(prepared, directory=True)
    receipt_file = private_path(prepared / "prepared-app.json")
    if receipt_file.stat().st_size > 65536:
        raise ValueError("prepared receipt exceeds metadata limit")
    receipt = json.loads(receipt_file.read_text())
    config = release_configuration()
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY).decode().strip()
    archive = private_path(prepared / "prepared-app.zip")
    if (receipt.get("commit") != commit or receipt.get("version") != config["version"]
            or receipt.get("productName") != config["productName"] or receipt.get("archiveSha256") != digest(archive)
            or not isinstance(receipt.get("notices"), dict)):
        raise ValueError("prepared app source/version/archive differs")
    bundle_name = config["productName"] + ".app"
    with zipfile.ZipFile(archive) as source:
        members = source.infolist()
        if len(members) > 100000 or sum(item.file_size for item in members) > 4 * 1024**3:
            raise ValueError("prepared archive exceeds expansion limit")
        for item in members:
            name = item.filename
            parts = name.rstrip("/").split("/")
            if (name.startswith("/") or any(p in ("", ".", "..") for p in parts)
                    or "\\" in name or parts[0] not in (bundle_name, "__MACOSX")):
                raise ValueError("unsafe prepared archive member")
            if stat.S_ISLNK(item.external_attr >> 16):
                if item.file_size > 4096:
                    raise ValueError("prepared link exceeds limit")
                target = source.read(item).decode("utf8")
                resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
                if target.startswith("/") or "\\" in target or not resolved.startswith(bundle_name + "/"):
                    raise ValueError("prepared archive link escapes app")
    return receipt


def prepare(output, environment):
    """Compile without signing credentials; hand off a verified app to a later job."""
    if sys.platform != "darwin":
        raise ValueError("preparation requires macOS")
    if not output.is_absolute() or output.exists() or output.is_symlink():
        raise ValueError("preparation output must be a new absolute directory")
    private_path(output.parent, directory=True)
    config = release_configuration()
    output.mkdir(mode=0o700)
    cache = output / "build-cache"
    # The beforeBuildCommand collects notices with offline Cargo metadata.
    # Preparation runs on a fresh runner, independently of the validation jobs.
    command(["cargo", "fetch", "--locked"], cwd=REPOSITORY / "app/src-tauri",
            env=build_environment(environment, cache))
    stdout, stderr = command([str(REPOSITORY / "app/node_modules/.bin/tauri"), "build",
                             "--config", "src-tauri/tauri.release.conf.json", "--", "--locked"],
                            cwd=REPOSITORY / "app", env=build_environment(environment, cache))
    (output / "preparation-build.log").write_bytes(stdout + stderr)
    app = cache / "release/bundle/macos" / (config["productName"] + ".app")
    layout(app, config["version"], config["productName"])
    notices = verify_notices(app)
    verify_asr_runtime(app)
    verify_inference_runtime(app)
    archive = output / "prepared-app.zip"
    command(["/usr/bin/ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(archive)], timeout=300)
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY).decode().strip()
    result = {"schemaVersion": 1, "commit": commit, "version": config["version"],
              "archiveSha256": digest(archive), "productName": config["productName"], "notices": notices}
    (output / "prepared-app.json").write_text(json.dumps(result, indent=2) + "\n")
    shutil.rmtree(cache)
    return result


def build(output, identity, team, profile, environment, prepared=None):
    # All explicit prerequisites precede mkdir, compilation, signing or upload.
    credentials(identity, team, profile)
    if sys.platform != "darwin":
        raise ValueError("distribution build supports macOS only")
    if not output.is_absolute() or output.exists() or output.is_symlink():
        raise ValueError("output must be a new absolute directory; existing contents are never overwritten")
    private_path(output.parent, directory=True)
    if shutil.disk_usage(output.parent).free < 4 * 1024 ** 3:
        raise ValueError("at least 4GiB free is required for the temporary build and copies")
    config = release_configuration()
    version = config["version"]
    product_name = config["productName"]
    bundle_name = product_name + ".app"
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
        if prepared is None:
            stdout, stderr = command([str(REPOSITORY / "app/node_modules/.bin/tauri"), "build", "--config",
                     "src-tauri/tauri.release.conf.json", "--", "--locked"], cwd=REPOSITORY / "app",
                    env=build_environment(environment, cache))
            with (output / "preparation-build.log").open("xb") as stream:
                stream.write(stdout + stderr)
            built = cache / "release/bundle/macos" / bundle_name
        else:
            # No build commands run in the signing job. The same-run archive
            # and its recorded source/hash are checked before extraction.
            receipt = prepared_receipt(prepared)
            archive = prepared / "prepared-app.zip"
            command(["/usr/bin/ditto", "-x", "-k", str(archive), str(cache)], timeout=300)
            built = cache / bundle_name
        timings["buildSeconds"] = round(time.monotonic() - started, 3)
        capture, embedding = layout(built, version, product_name)
        app = output / bundle_name
        command(["/usr/bin/ditto", str(built), str(app)], timeout=120)
        capture, embedding = layout(app, version, product_name)
        verify_notices(app) if prepared is None else verify_notices(app, receipt["notices"])
        verify_asr_runtime(app)
        verify_inference_runtime(app)
        started = time.monotonic()
        sign_asr_runtime(app, identity, team, policy)
        sign_inference_runtime(app, identity, team, policy)
        sign(capture, identity, team, policy / "capture.plist")
        sign(embedding, identity, team, policy / "embedding.plist")
        sign(app, identity, team, policy / "main.plist")
        timings["signAndVerifySeconds"] = round(time.monotonic() - started, 3)
        requests = []
        if profile is not None:
            # Staple the nested app before constructing the final DMG. Never
            # rebuild or mutate a DMG after obtaining that DMG's ticket.
            archive = output / f"{product_name}-notary.zip"
            command(["/usr/bin/ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(archive)], timeout=120)
            requests.append(notarize(archive, profile))
            command(["/usr/bin/xcrun", "stapler", "staple", str(app)], timeout=180)
            command(["/usr/bin/xcrun", "stapler", "validate", str(app)], timeout=180)
        image_source = output / "image-source"
        image_source.mkdir(mode=0o700)
        command(["/usr/bin/ditto", str(app), str(image_source / bundle_name)], timeout=120)
        (image_source / "Applications").symlink_to("/Applications")
        dmg = output / f"{product_name}_{version}_aarch64.dmg"
        command(["/usr/bin/hdiutil", "create", "-volname", product_name, "-srcfolder", str(image_source),
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
        dependency_notices = verify_notices(app) if prepared is None else verify_notices(app, receipt["notices"])
        asr_runtime = verify_asr_runtime(app, team)
        verify_inference_runtime(app, team)
        # Only this newly-created cache is reclaimed, after all owned commands
        # have returned and final artifacts are verified. Failures preserve it.
        now = cache.lstat()
        if cache.is_symlink() or (now.st_dev, now.st_ino) != (cache_identity.st_dev, cache_identity.st_ino):
            raise ValueError("build cache identity changed; preserve instead of cleanup")
        shutil.rmtree(cache)
        result = {"schemaVersion": 1, "productName": product_name, "version": version, "teamId": team,
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
    parser.add_argument("--team-id")
    parser.add_argument("--prepared-app", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--prepare", action="store_true", help="compile an unsigned handoff, no credentials or uploads")
    mode.add_argument("--sign-only", action="store_true", help="no uploads; not notarized")
    mode.add_argument("--notary-profile", help="explicitly authorize two uploads using an existing notarytool keychain profile")
    args = parser.parse_args(argv)
    try:
        if args.prepare:
            if args.prepared_app or args.team_id:
                raise ValueError("prepare does not accept signing inputs")
            result = prepare(args.output_dir, os.environ)
        else:
            if args.team_id is None:
                raise ValueError("--team-id is required for signing")
            if args.prepared_app is None:
                result = build(args.output_dir, os.environ.get("APPLE_SIGNING_IDENTITY", ""),
                               args.team_id, args.notary_profile, os.environ)
            else:
                result = build(args.output_dir, os.environ.get("APPLE_SIGNING_IDENTITY", ""),
                               args.team_id, args.notary_profile, os.environ, args.prepared_app)
        print(json.dumps(result, ensure_ascii=False))
        return 0
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
