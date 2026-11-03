"""Fail-closed severity gate shared by PR, scheduled, and release scans."""
import argparse
import ast
from datetime import date, datetime, timezone
import json
from pathlib import Path
import re
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def request_json(url, data=None):
    payload = json.dumps(data).encode() if data is not None else None
    request = urllib.request.Request(url, payload, {"Content-Type": "application/json", "User-Agent": "MinutesOn-security"})
    with urllib.request.urlopen(request, timeout=60) as response:
        raw = response.read(16 * 1024 * 1024 + 1)
    if len(raw) > 16 * 1024 * 1024:
        raise ValueError("security service response exceeded limit")
    return json.loads(raw)


def exceptions(root=ROOT, today=None):
    today = today or datetime.now(timezone.utc).date()
    result = json.loads((root / ".github/security-exceptions.json").read_text())["exceptions"]
    for item in result:
        if set(item) != {"tool", "id", "target", "reason", "owner", "created", "expires"}:
            raise ValueError("exception requires tool/id/target/reason/owner/created/expires")
        start, end = date.fromisoformat(item["created"]), date.fromisoformat(item["expires"])
        if not start <= today < end or not 0 < (end - start).days <= 30:
            raise ValueError(f"expired or invalid exception: {item['id']}")
        if item["tool"] not in ("osv", "codeql") or any(not isinstance(v, str) or not v.strip() for v in item.values()):
            raise ValueError("invalid or unscoped exception")
        if item["target"] == "*" or item["id"] == "*":
            raise ValueError("blanket exceptions are forbidden")
    return result


def score(advisory):
    from cvss import CVSS2, CVSS3, CVSS4
    values = []
    for severity in advisory.get("severity", []):
        vector = severity["score"]
        cls = {"CVSS_V2": CVSS2, "CVSS_V3": CVSS3, "CVSS_V4": CVSS4}.get(severity["type"])
        if cls:
            values.append(float(cls(vector).scores()[0]))
    for record in [advisory.get("database_specific", {}), *[a.get("database_specific", {}) for a in advisory.get("affected", [])]]:
        severity = record.get("severity", "").upper()
        if severity in {"LOW", "MODERATE", "MEDIUM", "HIGH", "CRITICAL"}:
            values.append({"LOW": 1, "MODERATE": 4, "MEDIUM": 4, "HIGH": 7, "CRITICAL": 9}[severity])
    return max(values) if values else None


def native_dependencies(root=ROOT):
    """Read literal definitions, never import/execute build scripts or download models."""
    result = {}
    for filename in ("mon_whisper_runtime.py", "mon_inference_runtime.py"):
        constants = {}
        for node in ast.parse((root / "tools" / filename).read_text()).body:
            if isinstance(node, ast.Assign) and isinstance(node.value, ast.Constant):
                for target in node.targets:
                    if isinstance(target, ast.Name):
                        constants[target.id] = node.value.value
        for key, repository in (("COMMIT", "ggml-org/whisper.cpp"), ("LLAMA_COMMIT", "ggml-org/llama.cpp"), ("SHERPA_COMMIT", "k2-fsa/sherpa-onnx")):
            if key in constants:
                commit = constants[key]
                if not re.fullmatch(r"[0-9a-f]{40}", commit):
                    raise ValueError("native source commit is malformed")
                result[(repository, commit)] = {"repository": repository, "commit": commit, "source": f"tools/{filename}"}
    # Include the prebuilt versions actually selected by runtime/model recipes.
    inputs = list((root / "app/model-catalog").glob("*.json")) + [root / "tools/mon_windows_runtime.py", root / "tools/mon_windows_capture_dependencies.py"]
    for file in inputs:
        for repository, tag in re.findall(r"https://github\.com/([^/\s\"']+/[^/\s\"']+)/releases/download/([^/\s\"']+)", file.read_text()):
            result[(repository, tag)] = {"repository": repository, "tag": tag, "source": file.relative_to(root).as_posix()}
        for tag in re.findall(r"raw\.githubusercontent\.com/nlohmann/json/(v[\d.]+)/", file.read_text()):
            result[("nlohmann/json", tag)] = {"repository": "nlohmann/json", "tag": tag, "source": file.relative_to(root).as_posix()}
    if not result:
        raise ValueError("native dependency inventory is empty")
    return list(result.values())


def osv_findings(data):
    findings = []
    if not isinstance(data.get("results"), list):
        raise ValueError("OSV report has no results array")
    for result in data["results"]:
        for package in result.get("packages", []):
            identity = package["package"]
            target = f"{identity.get('ecosystem', 'GIT')}:{identity['name']}@{identity['version']}"
            groups = []
            for advisory in package.get("vulnerabilities", []):
                if advisory.get("withdrawn"):
                    continue
                ids = {advisory["id"], *advisory.get("aliases", [])}
                merged, separate = [advisory], []
                for known_ids, records in groups:
                    if known_ids & ids:
                        ids |= known_ids
                        merged.extend(records)
                    else:
                        separate.append((known_ids, records))
                # A record can bridge two previously separate alias groups.
                changed = True
                while changed:
                    changed = False
                    for known_ids, records in list(separate):
                        if known_ids & ids:
                            ids |= known_ids
                            merged.extend(records)
                            separate.remove((known_ids, records))
                            changed = True
                groups = [*separate, (ids, merged)]
            for ids, records in groups:
                values = [value for a in records if (value := score(a)) is not None]
                # Maintenance notices are reportable, but are not unscored
                # vulnerabilities. Unsoundness advisories remain security issues.
                informational = all(a.get("affected") and all(
                    item.get("database_specific", {}).get("informational") in ("unmaintained", "notice")
                    for item in a["affected"]) for a in records)
                ordered = sorted(ids)
                findings.append({"tool": "osv", "id": ordered[0], "aliases": ordered[1:], "target": target,
                                 "score": max(values) if values else None, "informational": informational})
    return findings


def sarif_findings(data):
    findings = []
    if not isinstance(data.get("runs"), list) or not data["runs"]:
        raise ValueError("CodeQL SARIF has no runs")
    for run in data["runs"]:
        rules = {r["id"]: r for r in run["tool"]["driver"].get("rules", [])}
        for extension in run["tool"].get("extensions", []):
            rules.update({r["id"]: r for r in extension.get("rules", [])})
        for result in run.get("results", []):
            if result.get("kind") in ("pass", "notApplicable"):
                continue
            rule = rules[result["ruleId"]]
            properties = rule.get("properties", {})
            value = properties.get("security-severity")
            # Quality-only diagnostics have no security severity and remain warnings.
            if value is None and "security" not in properties.get("tags", []):
                continue
            locations = result.get("locations", [])
            target = locations[0]["physicalLocation"]["artifactLocation"]["uri"] if locations else "unknown"
            findings.append({"tool": "codeql", "id": result["ruleId"], "aliases": [], "target": target, "score": float(value) if value is not None else None})
    return findings


def blocked(findings, allowed):
    result = []
    for item in findings:
        if item.get("informational"):
            continue
        if item["score"] is not None and item["score"] < 7:
            continue
        ids = {item["id"], *item.get("aliases", [])}
        if not any(e["tool"] == item["tool"] and e["id"] in ids and e["target"] == item["target"] for e in allowed):
            result.append(item)
    return result


def scan(output, root=ROOT):
    output.mkdir(parents=True, exist_ok=True)
    report = output / "osv.json"
    process = subprocess.run([str(root / ".cache/security-tools/osv-scanner"), "scan", "source", "--no-call-analysis=all", "--format=json", f"--output-file={report}", "--lockfile=Cargo.lock", "--lockfile=app/package-lock.json"], cwd=root)
    if process.returncode not in (0, 1) or not report.is_file():
        raise ValueError("OSV scanner failed; no valid report")
    data = json.loads(report.read_text())
    native = native_dependencies(root)
    for item in native:
        query = {"commit": item["commit"]} if "commit" in item else {"package": {"ecosystem": "GIT", "name": "https://github.com/" + item["repository"]}, "version": item["tag"]}
        advisories, page = [], None
        while True:
            response = request_json("https://api.osv.dev/v1/query", {**query, **({"page_token": page} if page else {})})
            advisories.extend(response.get("vulns", []))
            page = response.get("next_page_token")
            if not page:
                break
        data["results"].append({"source": {"path": item["source"], "type": "git"}, "packages": [{"package": {"ecosystem": "GIT", "name": item["repository"], "version": item.get("commit", item.get("tag"))}, "vulnerabilities": advisories}]})
    (output / "native-dependencies.json").write_text(json.dumps({"components": native, "limitations": ["OSV commit/tag coverage is incomplete", "SDK/CRT system components and model weights are not vulnerability-scanned"]}, indent=2) + "\n")
    report.write_text(json.dumps(data, indent=2) + "\n")
    return osv_findings(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("scan", "sarif", "exceptions"))
    parser.add_argument("--output", type=Path, default=ROOT / "output/security")
    parser.add_argument("--sarif", type=Path)
    args = parser.parse_args()
    try:
        allowed = exceptions()
        findings = scan(args.output) if args.mode == "scan" else []
        if args.mode == "sarif":
            paths = sorted(args.sarif.rglob("*.sarif")) if args.sarif and args.sarif.is_dir() else [args.sarif]
            if not paths or any(p is None or not p.is_file() for p in paths):
                raise ValueError("CodeQL did not produce SARIF")
            for path in paths:
                findings.extend(sarif_findings(json.loads(path.read_text())))
        failures = blocked(findings, allowed)
        args.output.mkdir(parents=True, exist_ok=True)
        name = "codeql-gate.json" if args.mode == "sarif" else "dependency-gate.json"
        (args.output / name).write_text(json.dumps({"findings": findings, "blocking": failures}, indent=2) + "\n")
        print(f"{len(findings)} findings; {len(failures)} block (CVSS >= 7 or unknown)")
        return bool(failures)
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as error:
        print(f"Security scan failed: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
