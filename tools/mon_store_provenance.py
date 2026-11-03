"""Authenticate Store inputs using GitHub-verified release manifest provenance."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
from urllib.parse import quote

WORKFLOW = ".github/workflows/release-build.yml"
PUBLISH_STEP = "Revalidate draft, attach assets, and publish"
OIDC_ISSUER = "https://token.actions.githubusercontent.com"
PREDICATE = "https://slsa.dev/provenance/v1"
SHA = re.compile(r"[0-9a-f]{40}")
SHA256 = re.compile(r"[0-9a-f]{64}")
ERROR = "Store artifact provenance is missing or invalid"


def require(condition):
    if not condition:
        raise ValueError(ERROR)


def positive_integer(value):
    return type(value) is int and value > 0


def gh_json(arguments):
    result = subprocess.run(["gh", *arguments], check=True, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, timeout=120)
    require(len(result.stdout) <= 32 * 1024 * 1024)
    return json.loads(result.stdout)


def certificate_identity(result, repository, repository_id, manifest_hash):
    verified = result["verificationResult"]
    certificate = verified["signature"]["certificate"]
    repository_url = "https://github.com/" + repository
    workflow_url = repository_url + "/" + WORKFLOW + "@refs/heads/main"
    for key, expected in {
        "issuer": OIDC_ISSUER,
        "subjectAlternativeName": workflow_url,
        "buildSignerURI": workflow_url,
        "buildConfigURI": workflow_url,
        "sourceRepositoryURI": repository_url,
        "sourceRepositoryIdentifier": str(repository_id),
        "sourceRepositoryRef": "refs/heads/main",
        "buildTrigger": "workflow_dispatch",
    }.items():
        require(certificate.get(key) == expected)
    workflow_sha = certificate["sourceRepositoryDigest"]
    require(isinstance(workflow_sha, str) and SHA.fullmatch(workflow_sha))
    require(certificate.get("buildSignerDigest") == workflow_sha)
    require(certificate.get("buildConfigDigest") == workflow_sha)
    invocation = certificate["runInvocationURI"]
    require(isinstance(invocation, str))
    match = re.fullmatch(re.escape(repository_url) + r"/actions/runs/([1-9][0-9]*)/attempts/([1-9][0-9]*)", invocation)
    require(match is not None)
    # The predicate is workflow-controlled; only the verified certificate selects
    # the run and attempt. The verified subject binds the actual manifest bytes.
    statement = verified["statement"]
    require(statement["predicateType"] == PREDICATE)
    subjects = statement["subject"]
    require(isinstance(subjects, list) and all(isinstance(item, dict) for item in subjects))
    require(any(isinstance(item.get("digest"), dict) and item["digest"].get("sha256") == manifest_hash for item in subjects))
    return int(match[1]), int(match[2]), workflow_sha


def publication_complete(github, repository_id, workflow_id, identity):
    run_id, attempt, workflow_sha = identity
    endpoint = f"/actions/runs/{run_id}/attempts/{attempt}"
    run = github.call(endpoint)
    require(type(run["id"]) is int and run["id"] == run_id)
    require(type(run["run_attempt"]) is int and run["run_attempt"] == attempt)
    require(type(run["workflow_id"]) is int and run["workflow_id"] == workflow_id)
    require(run["path"] in (WORKFLOW, WORKFLOW + "@main", WORKFLOW + "@refs/heads/main"))
    require(run["event"] == "workflow_dispatch" and run["head_branch"] == "main")
    require(run["head_sha"] == workflow_sha)
    for key in ("repository", "head_repository"):
        require(type(run[key]["id"]) is int and run[key]["id"] == repository_id)
    if run["status"] in ("queued", "in_progress", "waiting", "pending", "requested"):
        require(run["conclusion"] is None)
        return False
    require(run["status"] == "completed" and run["conclusion"] == "success")
    jobs, total, ids = [], None, set()
    for page in range(1, 101):
        result = github.call(endpoint + f"/jobs?per_page=100&page={page}")
        count, batch = result["total_count"], result["jobs"]
        require(type(count) is int and 0 <= count <= 10000)
        require(total is None or count == total)
        total = count
        require(isinstance(batch, list) and len(batch) <= 100)
        for job in batch:
            require(isinstance(job, dict) and positive_integer(job["id"]))
            require(job["id"] not in ids)
            ids.add(job["id"])
        jobs.extend(batch)
        require(len(jobs) <= total)
        if len(jobs) == total:
            break
        require(len(batch) == 100)
    else:
        raise ValueError(ERROR)
    publications = [job for job in jobs if job.get("name") == "publish"]
    require(len(publications) == 1)
    job = publications[0]
    require(type(job["run_id"]) is int and job["run_id"] == run_id)
    # Some API versions omit run_attempt on jobs; the attempt-specific endpoint
    # above still excludes jobs from other attempts.
    if "run_attempt" in job:
        require(type(job["run_attempt"]) is int and job["run_attempt"] == attempt)
    require(job["head_sha"] == workflow_sha)
    require(job["status"] == "completed" and job["conclusion"] == "success")
    steps = job["steps"]
    require(isinstance(steps, list) and all(isinstance(step, dict) for step in steps))
    publications = [step for step in steps if step.get("name") == PUBLISH_STEP]
    require(len(publications) == 1)
    require(publications[0].get("status") == "completed" and publications[0].get("conclusion") == "success")
    return True


def store_receipt(github, release, manifest):
    require(type(manifest["schemaVersion"]) is int and manifest["schemaVersion"] == 1)
    source = manifest["sourceSha"]
    require(isinstance(source, str) and SHA.fullmatch(source))
    require(manifest["tag"] == release["tag_name"])
    tag = github.call("/commits/" + quote("refs/tags/" + release["tag_name"], safe=""))
    require(tag["sha"] == source)
    main = github.call("/commits/" + quote("refs/heads/main", safe=""))["sha"]
    require(isinstance(main, str) and SHA.fullmatch(main))
    # GitHub.call rejects '..' in paths; use gh for this API's required separator.
    comparison = gh_json(["api", "--hostname", "github.com",
                          f"repos/{github.repository}/compare/{source}...{main}"])
    require(comparison["status"] in ("ahead", "identical"))
    artifacts = manifest["artifacts"]
    require(isinstance(artifacts, list) and all(isinstance(item, dict) for item in artifacts))
    receipts = [item for item in artifacts if item.get("flavor") == "store"]
    require(len(receipts) == 1)
    receipt = receipts[0]
    require(type(receipt["schemaVersion"]) is int and receipt["schemaVersion"] == 1)
    require(receipt["sourceSha"] == source and receipt["verification"] == "store_payload_verified_unsigned")
    name = receipt["binary"]
    require(isinstance(name, str) and Path(name).name == name and "\\" not in name and "\x00" not in name and name.endswith(".msix"))
    require(isinstance(receipt["sha256"], str) and SHA256.fullmatch(receipt["sha256"]))
    require(positive_integer(receipt["bytes"]))
    return receipt


def verify_manifest(github, release, manifest_file):
    """Return (signed receipt, authenticated run ID), or None while publishing."""
    try:
        require(manifest_file.is_file() and not manifest_file.is_symlink())
        require(manifest_file.stat().st_size <= 32 * 1024 * 1024)
        raw = manifest_file.read_bytes()
        manifest_hash = hashlib.sha256(raw).hexdigest()
        identity_url = "https://github.com/" + github.repository + "/" + WORKFLOW + "@refs/heads/main"
        attestations = gh_json([
            "attestation", "verify", str(manifest_file), "--repo", github.repository,
            "--hostname", "github.com", "--cert-identity", identity_url,
            "--cert-oidc-issuer", OIDC_ISSUER, "--source-ref", "refs/heads/main",
            "--signer-workflow", github.repository + "/" + WORKFLOW,
            "--deny-self-hosted-runners", "--format", "json",
        ])
        require(manifest_file.read_bytes() == raw)
        require(isinstance(attestations, list) and 0 < len(attestations) <= 30)
        repository = gh_json(["api", "--hostname", "github.com", "repos/" + github.repository])
        repository_id = repository["id"]
        require(positive_integer(repository_id) and repository["full_name"] == github.repository)
        workflow = github.call("/actions/workflows/release-build.yml")
        workflow_id = workflow["id"]
        require(positive_integer(workflow_id) and workflow["path"] == WORKFLOW)
        receipt = store_receipt(github, release, json.loads(raw))
        waiting, seen = False, set()
        for attestation in attestations:
            try:
                identity = certificate_identity(attestation, github.repository, repository_id, manifest_hash)
                if identity in seen:
                    continue
                seen.add(identity)
                if publication_complete(github, repository_id, workflow_id, identity):
                    return receipt, identity[0]
                waiting = True
            except (ValueError, KeyError, TypeError, AttributeError):
                # Another verified attestation may be the completed publication,
                # e.g. when a dry run and production run signed identical bytes.
                continue
        if waiting:
            return None
        raise ValueError(ERROR)
    except (ValueError, KeyError, TypeError, AttributeError, OSError, subprocess.SubprocessError):
        raise ValueError(ERROR) from None
