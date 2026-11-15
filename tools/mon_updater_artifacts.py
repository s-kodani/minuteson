"""Create and verify Tauri updater assets from the final signed, stapled app."""
import base64
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ENDPOINT = "https://github.com/s-kodani/minuteson/releases/latest/download/latest.json"


def public_key(environment):
    key = environment.get("TAURI_UPDATER_PUBLIC_KEY", "").strip()
    try:
        lines = base64.b64decode(key, validate=True).decode().splitlines()
        raw = base64.b64decode(lines[1], validate=True)
        if not lines[0].startswith("untrusted comment:") or len(raw) != 42 or raw[:2] != b"Ed":
            raise ValueError()
    except (ValueError, IndexError, UnicodeError):
        raise ValueError("TAURI_UPDATER_PUBLIC_KEY must contain a Tauri public key") from None
    return key


def configuration(environment):
    config = json.loads(environment.get("TAURI_CONFIG") or "{}")
    config.setdefault("plugins", {})["updater"] = {
        "pubkey": public_key(environment), "endpoints": [ENDPOINT], "requireSignedVersion": True,
    }
    return json.dumps(config)


def verify(archive, signature, key, version):
    # A dedicated small binary avoids compiling or launching the desktop app in signing jobs.
    binary = ROOT / "app/src-tauri/target/debug/mon-update-verify"
    with tempfile.TemporaryDirectory(prefix="mon-update-verify-") as temporary:
        key_file = Path(temporary) / "public-key"
        key_file.write_text(key)
        subprocess.run([str(binary), str(key_file), str(signature), str(archive), version], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def validate_bundle_archive(archive, bundle_name):
    # Prove the archive preserves code signatures and the stapled notarization ticket.
    with tempfile.TemporaryDirectory(prefix="mon-update-bundle-") as temporary:
        with tarfile.open(archive) as source:
            source.extractall(temporary, filter="data")
        app = Path(temporary) / bundle_name
        for command in (["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)],
                        ["/usr/bin/xcrun", "stapler", "validate", str(app)]):
            subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def create(app, version, environment):
    key = public_key(environment)
    if not environment.get("TAURI_SIGNING_PRIVATE_KEY"):
        raise ValueError("TAURI_SIGNING_PRIVATE_KEY is required")
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("stable updater version required")
    archive = app.parent / f"MinutesOn_{version}_aarch64.app.tar.gz"
    # Tauri's macOS installer strips the first path component. Preserve executable modes and links.
    with tarfile.open(archive, "x:gz", format=tarfile.PAX_FORMAT) as output:
        output.add(app, arcname=app.name)
    validate_bundle_archive(archive, app.name)
    subprocess.run([str(ROOT / "app/node_modules/.bin/tauri"), "signer", "sign", "--app-version", version, str(archive)],
                   env=environment, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    signature = archive.with_name(archive.name + ".sig")
    verify(archive, signature, key, version)
    metadata = {"version": version, "platforms": {"darwin-aarch64": {
        "signature": signature.read_text().strip(),
        "url": f"https://github.com/s-kodani/minuteson/releases/download/v{version}/{archive.name}",
    }}}
    (app.parent / "latest.json").write_text(json.dumps(metadata, indent=2) + "\n")
    return [archive, signature, app.parent / "latest.json"]


def validate_metadata(directory, version):
    name = f"MinutesOn_{version}_aarch64.app.tar.gz"
    metadata = json.loads((directory / "latest.json").read_text())
    expected = {"version": version, "platforms": {"darwin-aarch64": {
        "signature": (directory / (name + ".sig")).read_text().strip(),
        "url": f"https://github.com/s-kodani/minuteson/releases/download/v{version}/{name}",
    }}}
    if metadata != expected:
        raise ValueError("updater metadata does not match the release")
    return [name, name + ".sig", "latest.json"]
