"""Stage exact build outputs and assemble a source-bound public release manifest."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
from urllib.parse import quote

from mon_security import native_dependencies

ROOT = Path(__file__).resolve().parents[1]


def digest(file):
    with file.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def one(pattern):
    values = list(ROOT.glob(pattern))
    if len(values) != 1 or not values[0].is_file() or values[0].is_symlink():
        raise ValueError(f"expected exactly one regular build artifact: {pattern}")
    return values[0]


def sbom(inventory, flavor, commit):
    components = []
    for package in inventory["packages"]:
        ecosystem = "cargo" if package["ecosystem"] == "cargo" else "npm"
        purl = f"pkg:{ecosystem}/{quote(package['name'], safe='/')}@{quote(package['version'], safe='')}"
        components.append({"type": "library", "bom-ref": purl, "name": package["name"], "version": package["version"], "purl": purl})
    for index, item in enumerate(native_dependencies()):
        components.append({"type": "library", "bom-ref": f"native-{index}", "name": item["repository"],
                           "version": item.get("commit", item.get("tag")), "externalReferences": [{"type": "vcs", "url": "https://github.com/" + item["repository"]}],
                           "properties": [{"name": "mon:source-definition", "value": item["source"]}, {"name": "mon:scope", "value": "build/runtime catalog; may include optional downloads"}]})
    return {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1,
            "metadata": {"component": {"type": "application", "name": "MinutesOn", "version": json.loads((ROOT / "app/package.json").read_text())["version"]},
                         "properties": [{"name": "mon:source-sha", "value": commit}, {"name": "mon:target", "value": flavor},
                                        {"name": "mon:scope", "value": inventory["scope"]},
                                        {"name": "mon:limitations", "value": "Rust build dependencies included; SDK/CRT/system libraries and model weights excluded; native catalog may include optional downloads"}]},
            "components": components}


def collect(flavor, output):
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip()
    if flavor == "macos":
        binary = one("output/macos-release/*.dmg")
        receipt = json.loads(one("output/macos-release/build-result.json").read_text())
        if receipt.get("notarization") != "tickets_validated" or len(receipt.get("notaryRequestIds", [])) != 2 or receipt.get("dmgSha256") != digest(binary):
            raise ValueError("DMG notarization/hash is not verified")
        # The unsigned build job retained its inventory inside the app handoff;
        # after signing use that exact validated bundle's inventory.
        inventory = one("output/macos-release/MinutesOn.app/Contents/Resources/third-party-inventory.json")
        status = "developer_id_signed_and_notarized"
    elif flavor == "windows":
        binary = one("output/windows-unsigned/MinutesOn-unsigned-setup.exe")
        receipt = json.loads(one("output/windows-unsigned-work/*-build.json").read_text())
        if receipt["source"]["commit"] != commit or receipt["artifacts"]["installer"]["sha256"] != digest(binary):
            raise ValueError("Windows build source/hash differs")
        unsigned = json.loads(one("output/windows-unsigned-work/*-unsigned.json").read_text(encoding="utf-8-sig"))
        if not unsigned:
            raise ValueError("unsigned verification evidence is missing")
        inventory = one(".cache/windows-third-party-notices/third-party-inventory.json")
        status = "unsigned_verified"
    else:
        binary = one("output/windows-store/*.msix")
        receipt = json.loads(one("output/windows-store-work/*/build.json").read_text())
        subprocess.run(["python", "tools/mon_windows_msix.py", "verify", "--manifest", str(one("output/windows-store-work/*/build.json")), "--package", str(binary), "--require-current-source"], cwd=ROOT, check=True)
        inventory = one(".cache/windows-third-party-notices/third-party-inventory.json")
        status = "store_payload_verified_unsigned"
    output.mkdir(parents=True, exist_ok=True)
    copied = output / binary.name
    shutil.copyfile(binary, copied)
    bom = output / f"{flavor}.cdx.json"
    bom.write_text(json.dumps(sbom(json.loads(inventory.read_text()), flavor, commit), ensure_ascii=False, indent=2) + "\n")
    record = {"schemaVersion": 1, "flavor": flavor, "sourceSha": commit, "binary": binary.name,
              "sha256": digest(copied), "bytes": copied.stat().st_size, "sbom": bom.name, "sbomSha256": digest(bom), "verification": status,
              "runtimeQA": "not_verified", "licenseReview": "not_verified"}
    (output / f"{flavor}-receipt.json").write_text(json.dumps(record, indent=2) + "\n")


def assemble(source, output, commit, tag):
    from mon_release import validate_versions
    validate_versions(tag, lambda name: (ROOT / name).read_text())
    output.mkdir(parents=True, exist_ok=True)
    public, records = {}, []
    for flavor in ("macos", "windows", "store"):
        receipt = json.loads((source / f"{flavor}-receipt.json").read_text())
        if receipt["sourceSha"] != commit or receipt["flavor"] != flavor:
            raise ValueError("artifact belongs to a different source/target")
        expected_status = {"macos": "developer_id_signed_and_notarized", "windows": "unsigned_verified", "store": "store_payload_verified_unsigned"}[flavor]
        if receipt.get("verification") != expected_status:
            raise ValueError("artifact verification is missing or belongs to another flavor")
        for key, hash_key in (("binary", "sha256"), ("sbom", "sbomSha256")):
            name = receipt[key]
            file = source / name
            if Path(name).name != name or file.is_symlink() or digest(file) != receipt[hash_key]:
                raise ValueError("artifact path/hash differs from receipt")
            # MSIX stays in Actions for Partner Center, not direct installation.
            if flavor == "store" and key == "binary":
                continue
            shutil.copyfile(file, output / name)
            public[name] = receipt[hash_key]
        records.append(receipt)
    manifest = {"schemaVersion": 1, "sourceSha": commit, "tag": tag, "publicFiles": public, "artifacts": records,
                "runtimeQA": "not_verified", "licenseReview": "not_verified", "distributionReady": False}
    manifest_file = output / "release-manifest.json"
    manifest_file.write_text(json.dumps(manifest, indent=2) + "\n")
    sums = {**public, manifest_file.name: digest(manifest_file)}
    (output / "SHA256SUMS").write_text("".join(f"{value}  {name}\n" for name, value in sorted(sums.items())))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("collect", "assemble"))
    parser.add_argument("--flavor", choices=("macos", "windows", "store"))
    parser.add_argument("--input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-sha")
    parser.add_argument("--tag")
    args = parser.parse_args()
    if args.mode == "collect":
        collect(args.flavor, args.output)
    else:
        assemble(args.input, args.output, args.source_sha, args.tag)
