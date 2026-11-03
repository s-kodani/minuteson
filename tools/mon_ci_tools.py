"""Install reviewed CI binaries, verifying repository-pinned SHA-256 digests."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def install(downloads=None):
    destination = ROOT / ".cache/security-tools"
    destination.mkdir(parents=True, exist_ok=True)
    target = f"{platform.system().lower()}-{platform.machine().lower()}"
    for name, tool in json.loads((ROOT / "tools/security-tools.json").read_text()).items():
        asset = tool["platforms"][target]
        if downloads:
            data = (downloads / asset["asset"]).read_bytes()
        else:
            url = f"https://github.com/{tool['repository']}/releases/download/{tool['version']}/{asset['asset']}"
            with urllib.request.urlopen(url, timeout=120) as response:
                data = response.read(128 * 1024 * 1024 + 1)
        if len(data) > 128 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != asset["sha256"]:
            raise ValueError(f"{name}: binary checksum differs")
        if asset["asset"].endswith(".tar.gz"):
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
                member = archive.getmember(name)
                if not member.isfile() or member.size > 128 * 1024 * 1024:
                    raise ValueError("invalid binary archive member")
                data = archive.extractfile(member).read()
        output = destination / name
        if output.is_symlink():
            raise ValueError("linked tool destination")
        output.write_bytes(data)
        output.chmod(0o755)
    if os.environ.get("GITHUB_PATH"):
        with open(os.environ["GITHUB_PATH"], "a", encoding="utf8") as stream:
            stream.write(str(destination) + "\n")
    print(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--downloads", type=Path)
    install(parser.parse_args().downloads)
