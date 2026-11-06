"""Build an unsigned Microsoft Store MSIX, with source and payload evidence.

Never installs/launches the app, trusts certificates, signs files or submits to
Partner Center. The NSIS signing and publication paths remain independent.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import uuid
import xml.etree.ElementTree as ET
import zipfile

_spec = importlib.util.spec_from_file_location("msix_evidence", Path(__file__).with_name("mon_windows_build_evidence.py"))
evidence = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(evidence)
ROOT = Path(__file__).resolve().parents[1]
FOUNDATION = "http://schemas.microsoft.com/appx/manifest/foundation/windows10"
UAP = "http://schemas.microsoft.com/appx/manifest/uap/windows10"
UAP10 = "http://schemas.microsoft.com/appx/manifest/uap/windows10/10"
RESCAP = "http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
MIN_OS = "10.0.26100.0"
RUNTIME_FILES = (
    "catalog.json",
    "embedding-index.json",
    "runtime.tar",
    "mon-speaker-embedding.exe",
    "asr/notices/nlohmann-MIT.txt",
    "whisper/notices/whisper-MIT.txt",
)
RUNTIME_DIRS = ("workers",)
BASE_LOGOS = ("StoreLogo.png", "Square44x44Logo.png", "Square150x150Logo.png")
APP_ICON_SIZES = (16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256)
APP_ICON_LOGOS = tuple(f"Square44x44Logo.targetsize-{size}{form}.png"
                       for size in APP_ICON_SIZES
                       for form in ("", "_altform-unplated", "_altform-lightunplated"))
LOGOS = BASE_LOGOS + APP_ICON_LOGOS
PAYLOAD_VERSION = 2
GENERATED_FILES = {"[Content_Types].xml", "AppxBlockMap.xml"}


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def configuration(root):
    store = read_json(root / "app/windows-store.json")
    if not isinstance(store, dict) or set(store) != {"identityName", "publisher", "publisherDisplayName", "displayName"}:
        raise ValueError("Store identity must include identityName, publisher, publisherDisplayName and displayName")
    for key, value in store.items():
        if (not isinstance(value, str) or not value or value != value.strip()
                or len(value) > 8192 or any(ord(c) < 32 for c in value)):
            raise ValueError(f"Store identity field is missing or invalid: {key}")
    if not re.fullmatch(r"[A-Za-z0-9.-]{3,50}", store["identityName"]) or not store["publisher"].startswith("CN="):
        raise ValueError("Invalid Microsoft Store package Name or Publisher")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9 ._-]{0,99}", store["displayName"]):
        raise ValueError("Store display name must also be a safe artifact filename")
    base = read_json(root / "app/src-tauri/tauri.conf.json")
    if base["productName"] != store["displayName"]:
        raise ValueError("Store and application display names must match")
    version = base["version"]
    if not isinstance(version, str) or not re.fullmatch(r"[1-9][0-9]*\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version):
        raise ValueError("Store version must be a numeric release with a nonzero major version")
    if any(int(n) > 65535 for n in version.split(".")):
        raise ValueError("Store version components must not exceed 65535")
    overlay = read_json(root / "app/src-tauri/tauri.windows.store.conf.json")
    qa = read_json(root / "app/src-tauri/tauri.qa.conf.json")
    if overlay["identifier"] != "jp.mon.desktop.store" or overlay["identifier"] in (base["identifier"], qa["identifier"]):
        raise ValueError("Store data and single-instance identity must be isolated")
    return {**store, "version": version + ".0", "architecture": "x64", "minimumOsVersion": MIN_OS,
            "tauriIdentifier": overlay["identifier"]}


def sdk_tool(name):
    sdk = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Windows Kits/10/bin"
    versions = sorted((p for p in sdk.glob("*") if re.fullmatch(r"\d+\.\d+\.\d+\.\d+", p.name)),
                      key=lambda p: tuple(map(int, p.name.split("."))), reverse=True)
    for version in versions:
        tool = version / "x64" / name
        if tool.is_file():
            return tool
    raise ValueError(f"Windows SDK x64 {name} is required")


def manifest_xml(config):
    for prefix, uri in (("", FOUNDATION), ("uap", UAP), ("uap10", UAP10), ("rescap", RESCAP)):
        ET.register_namespace(prefix, uri)
    def add(parent, name, attributes=None, text=None):
        element = ET.SubElement(parent, name if name.startswith("{") else f"{{{FOUNDATION}}}{name}", attributes or {})
        element.text = text
        return element
    package = ET.Element(f"{{{FOUNDATION}}}Package", {"IgnorableNamespaces": "uap uap10 rescap"})
    add(package, "Identity", {"Name": config["identityName"], "Publisher": config["publisher"],
                              "Version": config["version"], "ProcessorArchitecture": "x64"})
    properties = add(package, "Properties")
    for key, value in (("DisplayName", config["displayName"]), ("PublisherDisplayName", config["publisherDisplayName"]),
                       ("Description", "会議の録音・文字起こし・議事録生成"), ("Logo", "Assets\\StoreLogo.png")):
        add(properties, key, text=value)
    add(add(package, "Resources"), "Resource", {"Language": "ja-jp"})
    add(add(package, "Dependencies"), "TargetDeviceFamily", {"Name": "Windows.Desktop", "MinVersion": MIN_OS, "MaxVersionTested": MIN_OS})
    app = add(add(package, "Applications"), "Application", {"Id": "App", "Executable": "mon-desktop.exe",
              f"{{{UAP10}}}RuntimeBehavior": "packagedClassicApp", f"{{{UAP10}}}TrustLevel": "mediumIL"})
    add(app, f"{{{UAP}}}VisualElements", {"DisplayName": config["displayName"], "Description": config["displayName"],
        "Square150x150Logo": "Assets\\Square150x150Logo.png", "Square44x44Logo": "Assets\\Square44x44Logo.png", "BackgroundColor": "transparent"})
    capabilities = add(package, "Capabilities")
    add(capabilities, "Capability", {"Name": "internetClient"})
    add(capabilities, f"{{{RESCAP}}}Capability", {"Name": "runFullTrust"})
    add(capabilities, "DeviceCapability", {"Name": "microphone"})
    ET.indent(package, space="  ")
    return ET.tostring(package, encoding="utf-8", xml_declaration=True) + b"\n"


def regular(root, path):
    path = evidence.workspace_path(root, path)
    if not path.is_file() or path.is_symlink() or (hasattr(path, "is_junction") and path.is_junction()):
        raise ValueError(f"A regular build input is required: {path}")
    return path


def inventory(root, directory):
    directory = evidence.workspace_path(root, directory)
    result = []
    for path in sorted(directory.rglob("*")):
        evidence.workspace_path(root, path)
        if path.is_dir():
            continue
        regular(root, path)
        result.append({"path": path.relative_to(directory).as_posix(), "bytes": path.stat().st_size, "sha256": evidence.sha256_file(path)})
    return result


def stage(root, destination, config):
    destination = evidence.workspace_path(root, destination)
    if destination.exists():
        raise ValueError("MSIX staging requires a new directory")
    inputs = []
    def include(source, name):
        source = regular(root, source)
        if source.suffix.lower() == ".pdb":
            return
        inputs.append((source, name))
    include(root / "app/src-tauri/target/x86_64-pc-windows-msvc/release/mon-desktop.exe", "mon-desktop.exe")
    include(root / "app/src-tauri/binaries/mon-windows-capture-x86_64-pc-windows-msvc.exe", "mon-windows-capture.exe")
    runtime = root / ".cache/windows-runtime/package"
    for name in RUNTIME_FILES:
        include(runtime / name, "windows-runtime/" + name)
    for name in RUNTIME_DIRS:
        folder = evidence.workspace_path(root, runtime / name)
        if not folder.is_dir():
            raise ValueError(f"Missing Windows runtime directory: {name}")
        for path in sorted(folder.rglob("*")):
            evidence.workspace_path(root, path)
            if path.is_file():
                include(path, "windows-runtime/" + path.relative_to(runtime).as_posix())
    for name in LOGOS:
        include(root / "app/src-tauri/icons" / name, "Assets/" + name)
    include(root / ".cache/windows-capture-headers/LICENSE.MIT", "notices/nlohmann-json-MIT.txt")
    for name in ("THIRD_PARTY_NOTICES.txt", "third-party-inventory.json"):
        include(root / ".cache/windows-third-party-notices" / name, name)
    # All input presence/link checks precede staging. Compare copied bytes to
    # their source; the package will then be compared to this frozen stage.
    destination.mkdir(parents=True)
    for source, name in inputs:
        digest = evidence.sha256_file(source)
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        if evidence.sha256_file(target) != digest or evidence.sha256_file(source) != digest:
            raise ValueError(f"MSIX input changed while staging: {name}")
    (destination / "AppxManifest.xml").write_bytes(manifest_xml(config))
    return inventory(root, destination)


def verify_resource_index(dump, config):
    """Require shell icon variants under the manifest's logical asset path."""
    tree = ET.parse(dump)
    resource_map = tree.find("ResourceMap")
    if resource_map is None or resource_map.get("name") != config["identityName"]:
        raise ValueError("PRI resource map does not match the Store package identity")
    resources = resource_map.findall(".//NamedResource")
    prefix = f"ms-resource://{config['identityName']}/Files/Assets/"
    if {r.get("uri") for r in resources} != {prefix + n for n in BASE_LOGOS}:
        raise ValueError("PRI must index only the manifest's Assets resources")
    square = next(r for r in resources if r.get("uri") == prefix + "Square44x44Logo.png")
    expected = {("Assets/Square44x44Logo.png", "", "")}
    for size in APP_ICON_SIZES:
        for form in ("", "unplated", "lightunplated"):
            suffix = "_altform-" + form if form else ""
            expected.add((f"Assets/Square44x44Logo.targetsize-{size}{suffix}.png", str(size), form))
    actual = []
    for candidate in square.findall("Candidate"):
        qualifiers = {q.get("name"): q.get("value", "").lower()
                      for q in candidate.findall("QualifierSet/Qualifier")}
        actual.append((candidate.findtext("Value", "").replace("\\", "/"),
                       qualifiers.get("TargetSize", ""), qualifiers.get("AlternateForm", "")))
    if len(actual) != len(expected) or set(actual) != expected:
        raise ValueError("PRI is missing the complete default, unplated or lightunplated icon candidates")
    paths = [c.findtext("Value", "").replace("\\", "/") for c in resource_map.findall(".//Candidate")]
    if len(paths) != len(LOGOS) or set(paths) != {"Assets/" + n for n in LOGOS}:
        raise ValueError("PRI must index exactly the packaged icon assets")


def generate_resource_index(root, stage_path, config, work, makepri=None):
    """Index an Assets-only tree, preserving Files/Assets logical names.

    Starting an index at Assets would strip Assets from its logical resource
    names. A separate project root keeps the manifest paths and excludes the
    large inference runtime from resource indexing.
    """
    stage_path = evidence.workspace_path(root, stage_path)
    work = evidence.workspace_path(root, work)
    if work.is_relative_to(stage_path):
        raise ValueError("PRI work files must stay outside the package payload")
    makepri = makepri or sdk_tool("makepri.exe")
    sources = [(name, regular(root, stage_path / "Assets" / name)) for name in LOGOS]
    work.mkdir(parents=True, exist_ok=False)
    inputs = work / "input"
    (inputs / "Assets").mkdir(parents=True)
    for name, source in sources:
        shutil.copyfile(source, inputs / "Assets" / name)
    config_path = work / "priconfig.xml"
    subprocess.run([str(makepri), "createconfig", "/cf", str(config_path), "/dq", "lang-ja-JP", "/pv", "10.0.0"], check=True)
    tree = ET.parse(config_path)
    # All candidates belong to the main MSIX; do not emit resource packages.
    packaging = tree.find("packaging")
    if packaging is not None:
        tree.getroot().remove(packaging)
    tree.write(config_path, encoding="utf-8", xml_declaration=True)
    output = stage_path / "resources.pri"
    subprocess.run([str(makepri), "new", "/pr", str(inputs), "/cf", str(config_path),
                    "/in", config["identityName"], "/of", str(output)], check=True)
    regular(root, output)
    dump = work / "resources.pri.xml"
    subprocess.run([str(makepri), "dump", "/if", str(output), "/of", str(dump), "/dt", "detailed"], check=True)
    verify_resource_index(dump, config)
    return output


def verify_payload(package, config, files):
    expected = {}
    for entry in files:
        if (not isinstance(entry, dict) or set(entry) != {"path", "bytes", "sha256"}
                or not isinstance(entry["path"], str) or not entry["path"] or "\\" in entry["path"]
                or ":" in entry["path"] or entry["path"].startswith("/")
                or any(p in ("", ".", "..") for p in entry["path"].split("/"))
                or not isinstance(entry["bytes"], int) or entry["bytes"] < 0
                or not isinstance(entry["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"])):
            raise ValueError("Malformed MSIX payload evidence")
        name = entry["path"]
        if name.casefold() in {n.casefold() for n in expected} or PurePosixPath(name).suffix.lower() == ".pdb":
            raise ValueError("Duplicate or debug-only MSIX payload")
        expected[name] = entry
    required = {"AppxManifest.xml", "mon-desktop.exe", "mon-windows-capture.exe", "THIRD_PARTY_NOTICES.txt", "third-party-inventory.json", "notices/nlohmann-json-MIT.txt",
                *("Assets/" + n for n in BASE_LOGOS), *("windows-runtime/" + n for n in RUNTIME_FILES)}
    required.update({"resources.pri", *("Assets/" + n for n in APP_ICON_LOGOS)})
    if not required <= expected.keys():
        raise ValueError("Required MSIX payload is missing")
    with zipfile.ZipFile(package) as archive:
        entries = archive.infolist()
        names = [e.filename for e in entries]
        if len({n.casefold() for n in names}) != len(names) or set(names) != set(expected) | GENERATED_FILES:
            raise ValueError("MSIX contents differ from the staged payload")
        for name, entry in expected.items():
            info = archive.getinfo(name)
            if info.file_size != entry["bytes"]:
                raise ValueError(f"MSIX payload size mismatch: {name}")
            with archive.open(info) as source:
                digest = hashlib.file_digest(source, "sha256").hexdigest()
            if digest != entry["sha256"]:
                raise ValueError(f"MSIX payload hash mismatch: {name}")
        if archive.read("AppxManifest.xml") != manifest_xml(config):
            raise ValueError("MSIX Store identity or manifest differs")
        for name in ("mon-desktop.exe", "mon-windows-capture.exe"):
            with archive.open(name) as source:
                if source.read(2) != b"MZ":
                    raise ValueError(f"MSIX executable is invalid: {name}")
        blockmap = archive.read("AppxBlockMap.xml")
        if len(blockmap) > 32 * 1024 * 1024 or ET.fromstring(blockmap).get("HashMethod") != "http://www.w3.org/2001/04/xmlenc#sha256":
            raise ValueError("MSIX block map must use SHA-256")


def artifact(root, path):
    path = regular(root, path)
    return {"path": path.relative_to(root).as_posix(), "bytes": path.stat().st_size, "sha256": evidence.sha256_file(path)}


def finish(root, snapshot_path, stage_path, package, config, output):
    snapshot = read_json(snapshot_path)
    if snapshot.get("schemaVersion") != 1 or snapshot["source"] != evidence.source_identity(root):
        raise ValueError("Build source changed; previous MSIX retained")
    files = inventory(root, stage_path)
    verify_payload(package, config, files)
    result = {**snapshot, "kind": "windows-store-msix", "payloadVersion": PAYLOAD_VERSION,
              "completedAt": evidence.utc_now(), "identity": config,
              "package": artifact(root, package), "files": files, "signing": "microsoft-store-pending",
              "installedVerification": "pending", "certification": "pending", "distributionReady": False}
    evidence.save_new(root, output, result)
    return result


def verify(root, manifest_path, package=None, require_current_source=False):
    manifest = read_json(regular(root, manifest_path))
    if manifest.get("schemaVersion") != 1 or manifest.get("kind") != "windows-store-msix" or not manifest.get("completedAt"):
        raise ValueError("Completed MSIX build evidence is required")
    evidence.validate_source_identity(manifest["source"])
    matches = manifest["source"] == evidence.source_identity(root)
    if require_current_source and not matches:
        raise ValueError("Current source differs from the MSIX build; previous MSIX retained")
    expected = manifest["package"]
    if (not isinstance(expected, dict) or set(expected) != {"path", "bytes", "sha256"}
            or not isinstance(expected["path"], str) or not expected["path"]
            or "\\" in expected["path"] or ":" in expected["path"] or expected["path"].startswith("/")
            or any(p in ("", ".", "..") for p in expected["path"].split("/"))
            or not isinstance(expected["bytes"], int) or expected["bytes"] < 1
            or not isinstance(expected["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", expected["sha256"])):
        raise ValueError("Malformed MSIX package evidence")
    if package is not None:
        actual = regular(root, package)
    else:
        actual = regular(root, root / expected["path"])
        if artifact(root, actual) != expected:
            raise ValueError("MSIX package differs from its build evidence")
    if actual.stat().st_size != expected["bytes"] or evidence.sha256_file(actual) != expected["sha256"]:
        raise ValueError("Published MSIX differs from its build evidence")
    if manifest.get("payloadVersion") != PAYLOAD_VERSION:
        raise ValueError("Unsupported MSIX payload version")
    verify_payload(actual, manifest["identity"], manifest["files"])
    if require_current_source and manifest["identity"] != configuration(root):
        raise ValueError("Store configuration differs from this build")
    return {"currentSourceMatches": matches, "packageSha256": expected["sha256"], "payloadFiles": len(manifest["files"])}


def publish(root, manifest_path):
    verify(root, manifest_path, require_current_source=True)
    manifest = read_json(manifest_path)
    original = regular(root, root / manifest["package"]["path"])
    name = manifest["identity"]["displayName"] + "_" + manifest["identity"]["version"] + "_x64.msix"
    destination = evidence.workspace_path(root, root / "output/windows-store" / name)
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = evidence.workspace_path(root, original.parent / (".publish-" + uuid.uuid4().hex + ".msix"))
    try:
        shutil.copyfile(original, temporary)
        if evidence.sha256_file(temporary) != manifest["package"]["sha256"]:
            raise ValueError("MSIX copy verification failed; previous MSIX retained")
        if manifest["source"] != evidence.source_identity(root):
            raise ValueError("Source changed before publication; previous MSIX retained")
        os.replace(temporary, destination)
    finally:
        if temporary.exists():
            temporary.unlink()
    return destination


def build_environment(root=ROOT):
    environment = dict(os.environ, MON_WINDOWS_RELEASE_SIGNING="0", PYTHONUTF8="1")
    environment.pop("TAURI_CONFIG", None)
    environment["CARGO_TARGET_DIR"] = str(root / "app/src-tauri/target")
    environment["NPM_CONFIG_CACHE"] = str(root / ".cache/npm")
    # VS installs CMake outside PATH by default. Only augment this child build.
    if not shutil.which("cmake"):
        visual_studio = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Microsoft Visual Studio/2022"
        for installation in sorted(visual_studio.glob("*")):
            binary = installation / "Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin"
            if (binary / "cmake.exe").is_file():
                environment["PATH"] = str(binary) + os.pathsep + environment.get("PATH", "")
                break
    return environment


def build(root):
    if os.name != "nt":
        raise ValueError("Build MSIX packages on Windows")
    config = configuration(root)
    makeappx = sdk_tool("makeappx.exe")
    makepri = sdk_tool("makepri.exe")
    environment = build_environment(root)
    # CreateProcess resolves the executable before applying the child's PATH.
    # Use the discovered absolute path when CMake is installed outside PATH.
    ctest = shutil.which("ctest", path=environment.get("PATH", ""))
    if not ctest:
        raise ValueError("CMake/CTest is required (PATH or the Visual Studio 2022 CMake installation)")
    build_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex
    work = evidence.workspace_path(root, root / "output/windows-store-work" / build_id)
    snapshot = work / "source.json"
    evidence.begin(root, snapshot)
    temporary = work / "temp"
    temporary.mkdir()
    environment.update(TEMP=str(temporary), TMP=str(temporary))
    subprocess.run(["npm.cmd", "run", "tauri", "--", "build", "--no-bundle", "--target", "x86_64-pc-windows-msvc",
                    "--config", "src-tauri/tauri.windows.bundle.conf.json", "--config", "src-tauri/tauri.windows.store.conf.json",
                    "--", "--locked"], cwd=root / "app", env=environment, check=True)
    subprocess.run([ctest, "--test-dir", str(root / "probes/windows-capture/build"), "-C", "Release", "--output-on-failure", "--no-tests=error"],
                   cwd=root, env=environment, check=True)
    stage_path = work / "stage"
    stage(root, stage_path, config)
    generate_resource_index(root, stage_path, config, work / "pri", makepri)
    package = work / (config["displayName"] + "_" + config["version"] + "_x64.msix")
    subprocess.run([str(makeappx), "pack", "/d", str(stage_path), "/p", str(package), "/h", "SHA256", "/no"], check=True)
    manifest = work / "build.json"
    finish(root, snapshot, stage_path, package, config, manifest)
    destination = publish(root, manifest)
    print("MSIX:", destination)
    print("Build evidence:", manifest)
    print("SHA256:", evidence.sha256_file(destination))
    print("Store signing, installed verification and WACK certification: pending")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("build")
    commands.add_parser("validate")
    check = commands.add_parser("verify")
    check.add_argument("--manifest", type=Path, required=True)
    check.add_argument("--package", type=Path)
    check.add_argument("--require-current-source", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "build":
            build(ROOT)
        elif args.command == "validate":
            print(json.dumps({"identity": configuration(ROOT), "makeappx": str(sdk_tool("makeappx.exe")),
                              "makepri": str(sdk_tool("makepri.exe"))}, ensure_ascii=True, indent=2))
        else:
            print(json.dumps(verify(ROOT, args.manifest, args.package, args.require_current_source)))
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError, zipfile.BadZipFile, ET.ParseError) as error:
        parser.exit(1, f"MSIX build/verification failed: {error}\n")


if __name__ == "__main__":
    main()
