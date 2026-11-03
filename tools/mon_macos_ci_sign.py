"""Import CI credentials only after compilation; always remove the ephemeral keychain."""
import base64
import os
from pathlib import Path
import secrets
import shlex
import subprocess
import tempfile

import mon_macos_release as release


def command(*args):
    return release.command(list(args), timeout=180)[0]


def main():
    required = ("MACOS_CERTIFICATE_P12_BASE64", "MACOS_CERTIFICATE_PASSWORD", "APPLE_NOTARY_KEY_BASE64",
                "APPLE_NOTARY_KEY_ID", "APPLE_NOTARY_ISSUER_ID", "APPLE_TEAM_ID", "APPLE_SIGNING_IDENTITY")
    if any(not os.environ.get(key) for key in required):
        raise ValueError("macOS release credentials are incomplete")
    release.prepared_receipt((release.REPOSITORY / "output/prepared-macos").resolve())
    old_search = shlex.split(command("/usr/bin/security", "list-keychains", "-d", "user").decode())
    old_default = shlex.split(command("/usr/bin/security", "default-keychain", "-d", "user").decode())[0]
    with tempfile.TemporaryDirectory(prefix="mon-sign-", dir=os.environ["RUNNER_TEMP"]) as temporary:
        directory = Path(temporary)
        keychain = str(directory / "release.keychain-db")
        certificate, key = directory / "certificate.p12", directory / "notary.p8"
        for file, value in ((certificate, "MACOS_CERTIFICATE_P12_BASE64"), (key, "APPLE_NOTARY_KEY_BASE64")):
            with file.open("xb") as stream:
                os.fchmod(stream.fileno(), 0o600)
                stream.write(base64.b64decode(os.environ[value], validate=True))
        password = secrets.token_urlsafe(32)
        created = False
        try:
            command("/usr/bin/security", "create-keychain", "-p", password, keychain)
            created = True
            command("/usr/bin/security", "set-keychain-settings", "-lut", "21600", keychain)
            command("/usr/bin/security", "unlock-keychain", "-p", password, keychain)
            command("/usr/bin/security", "import", str(certificate), "-k", keychain,
                    "-P", os.environ["MACOS_CERTIFICATE_PASSWORD"], "-T", "/usr/bin/codesign")
            command("/usr/bin/security", "set-key-partition-list", "-S", "apple-tool:,apple:", "-s", "-k", password, keychain)
            command("/usr/bin/security", "list-keychains", "-d", "user", "-s", keychain, *old_search)
            command("/usr/bin/security", "default-keychain", "-d", "user", "-s", keychain)
            command("/usr/bin/xcrun", "notarytool", "store-credentials", "mon-ci",
                    "--key", str(key), "--key-id", os.environ["APPLE_NOTARY_KEY_ID"],
                    "--issuer", os.environ["APPLE_NOTARY_ISSUER_ID"], "--keychain", keychain)
            result = release.build((release.REPOSITORY / "output/macos-release").resolve(),
                                   os.environ["APPLE_SIGNING_IDENTITY"], os.environ["APPLE_TEAM_ID"], "mon-ci", os.environ,
                                   (release.REPOSITORY / "output/prepared-macos").resolve())
            if result["notarization"] != "tickets_validated":
                raise ValueError("DMG notarization did not complete")
        finally:
            # Do not short-circuit the remaining cleanup if one restore fails.
            operations = [["/usr/bin/security", "default-keychain", "-d", "user", "-s", old_default],
                          ["/usr/bin/security", "list-keychains", "-d", "user", "-s", *old_search]]
            if created:
                operations.append(["/usr/bin/security", "delete-keychain", keychain])
            cleanup_failed = False
            for operation in operations:
                try:
                    process = subprocess.run(operation, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
                    cleanup_failed |= process.returncode != 0
                except subprocess.SubprocessError:
                    cleanup_failed = True
            if cleanup_failed:
                raise ValueError("ephemeral signing keychain cleanup failed")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.SubprocessError):
        raise SystemExit("macOS CI signing failed (credentials and command output redacted)")
