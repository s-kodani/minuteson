"""Synthetic Store provenance boundary tests; no provider writes or network."""
import base64
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import mon_release as release
import mon_store_provenance as provenance
import mon_store_submission as store

SOURCE = "1" * 40
WORKFLOW_SOURCE = "2" * 40
MAIN = "3" * 40
PACKAGE = b"synthetic inert Store package"


class Fixture:
    def __init__(self):
        self.repository = "owner/repo"
        self.record = {"status": "published", "store": "pending", "sourceSha": SOURCE, "buildRunId": "999"}
        self.value = {"id": 123, "tag_name": "v1.2.3", "draft": False, "prerelease": False}
        self.save_state(self.value, self.record)
        self.manifest = {"schemaVersion": 1, "sourceSha": SOURCE, "tag": "v1.2.3", "artifacts": [{
            "schemaVersion": 1, "flavor": "store", "sourceSha": SOURCE,
            "binary": "MinutesOn_1.2.3.0_x64.msix", "sha256": hashlib.sha256(PACKAGE).hexdigest(),
            "bytes": len(PACKAGE), "verification": "store_payload_verified_unsigned",
        }]}
        self.repository_data = {"id": 1234, "full_name": self.repository}
        self.workflow = {"id": 5, "path": ".github/workflows/release-build.yml"}
        self.run = {"id": 42, "run_attempt": 2, "workflow_id": 5, "path": ".github/workflows/release-build.yml@main",
                    "event": "workflow_dispatch", "head_branch": "main", "head_sha": WORKFLOW_SOURCE,
                    "repository": {"id": 1234}, "head_repository": {"id": 1234},
                    "status": "completed", "conclusion": "success"}
        self.job = {"id": 7, "run_id": 42, "run_attempt": 2, "head_sha": WORKFLOW_SOURCE,
                    "name": "publish", "status": "completed", "conclusion": "success", "steps": [{
                        "name": "Revalidate draft, attach assets, and publish", "status": "completed", "conclusion": "success",
                    }]}
        self.jobs = [self.job]
        self.pages = None
        self.tag_sha, self.main_sha, self.compare_status = SOURCE, MAIN, "ahead"
        self.package = PACKAGE
        self.commands, self.requests = [], []
        self.verifier_error, self.verifier_output = None, None
        self.saved = []
        self.attestations = [self.attestation()]

    def attestation(self, run_id=42, attempt=2):
        workflow = "https://github.com/owner/repo/.github/workflows/release-build.yml@refs/heads/main"
        return {"verificationResult": {"signature": {"certificate": {
            "issuer": "https://token.actions.githubusercontent.com", "subjectAlternativeName": workflow,
            "buildSignerURI": workflow, "buildConfigURI": workflow,
            "sourceRepositoryURI": "https://github.com/owner/repo", "sourceRepositoryIdentifier": "1234",
            "sourceRepositoryRef": "refs/heads/main", "buildTrigger": "workflow_dispatch",
            "sourceRepositoryDigest": WORKFLOW_SOURCE, "buildSignerDigest": WORKFLOW_SOURCE,
            "buildConfigDigest": WORKFLOW_SOURCE,
            "runInvocationURI": f"https://github.com/owner/repo/actions/runs/{run_id}/attempts/{attempt}",
        }}, "statement": {"predicateType": "https://slsa.dev/provenance/v1", "subject": [],
                            "predicate": {"runDetails": {"metadata": {"invocationId": "https://github.com/attacker/repo/actions/runs/999/attempts/1"}}}}}}

    def main_candidate(self, event="push"):
        self.manifest.update(schemaVersion=2, producer={
            "kind": "main-build", "runId": 42, "runAttempt": 2, "event": event,
        })
        self.run.update(event=event, head_sha=SOURCE)
        self.job.update(name="candidate", head_sha=SOURCE, steps=[{
            "name": provenance.CANDIDATE_STEP, "status": "completed", "conclusion": "success",
        }])
        certificate = self.attestations[0]["verificationResult"]["signature"]["certificate"]
        certificate.update(buildTrigger=event, sourceRepositoryDigest=SOURCE,
                           buildSignerDigest=SOURCE, buildConfigDigest=SOURCE)
        return self

    def save_state(self, value, record):
        self.value = deepcopy(value)
        self.record = deepcopy(record)
        self.value["body"] = "<!-- mon-release:" + base64.b64encode(json.dumps(record).encode()).decode() + " -->"
        if hasattr(self, "saved"):
            self.saved.append(deepcopy(record))
        return deepcopy(self.value)

    def release(self, _):
        return deepcopy(self.value)

    def releases(self):
        return [self.release(123)]

    def call(self, endpoint):
        if not endpoint.startswith("/") or ".." in endpoint:
            raise ValueError("invalid GitHub endpoint")
        self.requests.append(endpoint)
        if endpoint == "/actions/workflows/release-build.yml":
            return deepcopy(self.workflow)
        if endpoint == "/actions/runs/42/attempts/2":
            return deepcopy(self.run)
        if endpoint.startswith("/actions/runs/42/attempts/2/jobs?"):
            page = int(endpoint.rsplit("=", 1)[1])
            if self.pages is not None:
                return deepcopy(self.pages[page - 1])
            return {"total_count": len(self.jobs), "jobs": deepcopy(self.jobs[(page - 1) * 100:page * 100])}
        if endpoint == "/commits/refs%2Ftags%2Fv1.2.3":
            return {"sha": self.tag_sha}
        if endpoint == "/commits/refs%2Fheads%2Fmain":
            return {"sha": self.main_sha}
        raise AssertionError(endpoint)

    def command(self, argv, **kwargs):
        self.commands.append((argv, kwargs))
        if argv[1:3] == ["release", "download"]:
            directory = Path(argv[argv.index("--dir") + 1])
            (directory / "release-manifest.json").write_text(json.dumps(self.manifest))
        elif argv[1] == "api":
            if argv[-1] == "repos/" + self.repository:
                data = self.repository_data
            elif argv[-1] == f"repos/{self.repository}/compare/{SOURCE}...{self.main_sha}":
                data = {"status": self.compare_status}
            else:
                raise AssertionError(argv)
            return SimpleNamespace(stdout=json.dumps(data))
        elif argv[1:3] == ["attestation", "verify"]:
            if self.verifier_error:
                raise self.verifier_error
            result = deepcopy(self.attestations)
            for attestation in result if isinstance(result, list) else []:
                if isinstance(attestation, dict) and "verificationResult" in attestation:
                    statement = attestation["verificationResult"].get("statement")
                    if isinstance(statement, dict) and statement.get("subject") == []:
                        statement["subject"] = [{"name": "release-manifest.json", "digest": {
                            "sha256": hashlib.sha256(Path(argv[3]).read_bytes()).hexdigest(),
                        }}]
            return SimpleNamespace(stdout=self.verifier_output if self.verifier_output is not None else json.dumps(result))
        elif argv[1:3] == ["run", "download"]:
            directory = Path(argv[argv.index("--dir") + 1])
            name = self.manifest["artifacts"][0]["binary"]
            (directory / name).write_bytes(self.package)
        else:
            raise AssertionError(argv)
        return SimpleNamespace(stdout="")

    def provider(self, pending=False):
        provider = Mock()
        submission = {"id": "S1", "applicationPackages": [], "fileUploadUrl": "https://example.blob.core.windows.net/upload"}
        def call(suffix="", data=None, method=None):
            if suffix.endswith("/status"):
                return {"status": "PendingCommit" if pending else "Published"}
            if not suffix:
                return {"lastPublishedApplicationSubmission": {"id": "old"}}
            if suffix in ("/submissions", "/submissions/S1"):
                return deepcopy(submission)
            return None
        provider.call.side_effect = call
        return provider


class StoreProvenanceTests(unittest.TestCase):
    def test_completed_main_candidate_can_submit_the_same_saved_package(self):
        for event in ("push", "workflow_dispatch"):
            fixture = Fixture().main_candidate(event)
            provider = self.run_boundary(fixture)
            provider.upload.assert_called_once()
            self.assertEqual(fixture.record["sourceSha"], SOURCE)
            self.assertEqual(fixture.record["store"], "CommitStarted")

    def test_main_candidate_cannot_select_a_different_run_attempt_source_or_step(self):
        for change in ({"runId": 999}, {"runAttempt": 3}, {"kind": "release-build"}, {"event": "pull_request"}):
            fixture = Fixture().main_candidate()
            fixture.manifest["producer"].update(change)
            self.assert_rejected(fixture)
        fixture = Fixture().main_candidate()
        fixture.job["steps"][0]["conclusion"] = "skipped"
        self.assert_rejected(fixture)
        fixture = Fixture().main_candidate()
        fixture.run["head_sha"] = WORKFLOW_SOURCE
        self.assert_rejected(fixture)

    def test_draft_candidate_proof_does_not_require_a_tag_until_manual_publication(self):
        fixture = Fixture().main_candidate()
        fixture.value.update(draft=True, target_commitish=SOURCE)
        with tempfile.TemporaryDirectory() as temporary, patch.object(store.subprocess, "run", side_effect=fixture.command):
            file = Path(temporary) / "release-manifest.json"
            file.write_text(json.dumps(fixture.manifest))
            self.assertEqual(provenance.verify_manifest(fixture, fixture.release(123), file, draft=True, expected_run=42)[1], 42)
        self.assertFalse(any("/commits/refs%2Ftags" in endpoint for endpoint in fixture.requests))

    def test_polling_pending_commit_and_existing_legacy_ids_never_resumes_writes(self):
        for status in ("PendingCommit", "Certification", "Published", "error"):
            fixture = Fixture()
            fixture.record.update(storeSubmissionId="S1", store=status)
            fixture.save_state(fixture.value, fixture.record)
            provider = fixture.provider()
            provider.call.side_effect = None
            provider.call.return_value = {"status": "PendingCommit" if status == "error" else status}
            with patch.object(store, "GitHub", return_value=fixture), patch.object(store, "Store", return_value=provider) as constructor, \
                    patch.dict(os.environ, {"MON_STORE_RELEASE_ID": "123" if status == "Published" else ""}), patch.object(store.subprocess, "run") as download:
                store.main("poll")
                constructor.assert_called_once_with(read_only=True)
                download.assert_not_called()
            provider.call.assert_called_once_with("/submissions/S1/status")
            provider.upload.assert_not_called()
            self.assertEqual(fixture.record["storeSubmissionId"], "S1")

    def test_read_only_store_client_rejects_every_mutating_api_and_upload(self):
        provider = object.__new__(store.Store)
        provider.read_only, provider.app, provider.token = True, "app", "inert"
        with patch.object(store, "http", return_value={"status": "PendingCommit"}) as http:
            provider.call("/submissions/S1/status")
            for suffix, data, method in (("/submissions", None, "POST"), ("/submissions/S1", {}, "PUT"),
                                          ("/submissions/S1/commit", None, "POST"), ("", None, None)):
                with self.assertRaises(ValueError): provider.call(suffix, data, method)
            with self.assertRaises(ValueError): provider.upload("https://example.blob.core.windows.net/upload", Path("unused"))
            http.assert_called_once()

    def run_boundary(self, fixture, provider=None):
        provider = provider or fixture.provider()
        with tempfile.TemporaryDirectory() as temporary, patch.object(store.subprocess, "run", side_effect=fixture.command):
            store.resume(fixture, provider, fixture.release(123), Path(temporary))
        return provider

    def assert_rejected(self, fixture, pending=False):
        provider = fixture.provider(pending=pending)
        if pending:
            fixture.record.update(storeSubmissionId="S1", storePackageSha256=hashlib.sha256(PACKAGE).hexdigest())
            fixture.save_state(fixture.value, fixture.record)
        with self.assertRaises(ValueError):
            self.run_boundary(fixture, provider)
        provider.upload.assert_not_called()
        self.assertFalse(any(call.kwargs.get("method") or (len(call.args) > 2 and call.args[2]) for call in provider.call.call_args_list))
        if not pending:
            provider.call.assert_not_called()

    def test_forged_release_state_and_consistent_manifest_cannot_submit(self):
        fixture = Fixture()
        fixture.verifier_error = subprocess.CalledProcessError(1, ["gh", "attestation", "verify"], stderr="untrusted details")
        for pending in (False, True):
            with self.subTest(pending=pending):
                self.assert_rejected(fixture, pending=pending)
        self.assertFalse(any(argv[1:3] == ["run", "download"] for argv, _ in fixture.commands))

    def test_verified_manifest_uses_certificate_run_and_allows_distinct_build_source(self):
        fixture = Fixture()
        fixture.record.update(sourceSha="attacker-controlled state", buildRunId="999")
        fixture.save_state(fixture.value, fixture.record)
        provider = self.run_boundary(fixture)
        provider.upload.assert_called_once()
        run_command = next(argv for argv, _ in fixture.commands if argv[1:3] == ["run", "download"])
        self.assertEqual(run_command[3], "42")
        self.assertEqual(fixture.record["store"], "CommitStarted")
        self.assertIn("/actions/runs/42/attempts/2", fixture.requests)
        self.assertFalse(any(endpoint == "/actions/runs/42" for endpoint in fixture.requests))
        self.assertIn("/commits/refs%2Ftags%2Fv1.2.3", fixture.requests)
        self.assertIn("/commits/refs%2Fheads%2Fmain", fixture.requests)
        command, options = next((argv, kwargs) for argv, kwargs in fixture.commands if argv[1:3] == ["attestation", "verify"])
        self.assertNotIn("--bundle", command)
        # gh treats these signer selectors as mutually exclusive. Keep the
        # exact main-workflow certificate identity as the single selector.
        selectors = {"--cert-identity", "--cert-identity-regex", "--signer-repo", "--signer-workflow"}
        self.assertEqual([argument for argument in command if argument in selectors], ["--cert-identity"])
        self.assertIn("--deny-self-hosted-runners", command)
        for flag, value in (("--repo", "owner/repo"), ("--cert-identity", "https://github.com/owner/repo/.github/workflows/release-build.yml@refs/heads/main"),
                            ("--cert-oidc-issuer", "https://token.actions.githubusercontent.com"), ("--source-ref", "refs/heads/main")):
            self.assertEqual(command[command.index(flag) + 1], value)
        self.assertTrue(options["check"])
        self.assertIs(options["stderr"], subprocess.PIPE)

    def test_invalid_certificate_identity_never_mutates_store(self):
        changes = {
            "issuer": "https://attacker.example", "subjectAlternativeName": "https://github.com/owner/repo/.github/workflows/release-build.yml@refs/heads/feature",
            "buildSignerURI": "https://github.com/owner/repo/.github/workflows/other.yml@refs/heads/main",
            "buildConfigURI": "https://github.com/owner/repo/.github/workflows/other.yml@refs/heads/main",
            "sourceRepositoryURI": "https://github.com/attacker/repo", "sourceRepositoryIdentifier": "999",
            "sourceRepositoryRef": "refs/tags/main", "buildTrigger": "pull_request",
            "sourceRepositoryDigest": "bad", "buildSignerDigest": SOURCE, "buildConfigDigest": SOURCE,
            "runInvocationURI": "https://github.com/attacker/repo/actions/runs/42/attempts/2",
        }
        for key, value in changes.items():
            with self.subTest(field=key):
                fixture = Fixture()
                fixture.attestations[0]["verificationResult"]["signature"]["certificate"][key] = value
                self.assert_rejected(fixture)

    def test_run_invocation_aliases_and_missing_attempt_are_rejected(self):
        suffixes = ["/actions/runs/42", "/actions/runs/42/attempts/0", "/actions/runs/042/attempts/2",
                    "/actions/runs/42/attempts/2?other=1", "/actions/runs/42/attempts/2#x", "/actions/runs/42/attempts/%32"]
        for suffix in suffixes:
            with self.subTest(suffix=suffix):
                fixture = Fixture()
                fixture.attestations[0]["verificationResult"]["signature"]["certificate"]["runInvocationURI"] = "https://github.com/owner/repo" + suffix
                self.assert_rejected(fixture)

    def test_certified_attempt_metadata_cannot_be_replaced_by_other_run_or_attempt(self):
        changes = {"id": 999, "run_attempt": 3, "workflow_id": 9, "path": ".github/workflows/other.yml",
                   "event": "push", "head_branch": "feature", "head_sha": SOURCE,
                   "repository": {"id": 999}, "head_repository": {"id": 999}, "conclusion": "failure"}
        for key, value in changes.items():
            with self.subTest(field=key):
                fixture = Fixture()
                fixture.run[key] = value
                self.assert_rejected(fixture)

    def test_dry_run_success_without_real_publication_is_rejected(self):
        for conclusion in ("skipped", "failure", None):
            with self.subTest(conclusion=conclusion):
                fixture = Fixture()
                fixture.job["steps"][0]["conclusion"] = conclusion
                self.assert_rejected(fixture)
        fixture = Fixture()
        fixture.job["steps"] = []
        self.assert_rejected(fixture)

    def test_pending_publication_waits_without_permanent_error_or_package_download(self):
        fixture = Fixture()
        fixture.run.update(status="in_progress", conclusion=None)
        provider = fixture.provider()
        with patch.dict(os.environ, {"MON_STORE_RELEASE_ID": "123", "GITHUB_REF": "refs/heads/main"}), patch.object(store, "GitHub", return_value=fixture), patch.object(store, "Store", return_value=provider), \
                patch.object(store.subprocess, "run", side_effect=fixture.command):
            store.main("submit")
        provider.call.assert_not_called()
        provider.upload.assert_not_called()
        self.assertEqual(fixture.record["store"], "pending")
        self.assertFalse(fixture.saved)
        self.assertFalse(any(argv[1:3] == ["run", "download"] for argv, _ in fixture.commands))

    def test_another_verified_attestation_can_prove_completed_publication(self):
        fixture = Fixture()
        wrong = fixture.attestation()
        wrong["verificationResult"]["signature"]["certificate"]["sourceRepositoryIdentifier"] = "999"
        fixture.attestations.insert(0, wrong)
        self.run_boundary(fixture).upload.assert_called_once()

    def test_skipped_earlier_attempt_cannot_mask_real_publication_in_another_attempt(self):
        fixture = Fixture()
        fixture.attestations.insert(0, fixture.attestation(attempt=1))
        original = fixture.call
        def call(endpoint):
            if endpoint == "/actions/runs/42/attempts/1":
                fixture.requests.append(endpoint)
                return dict(deepcopy(fixture.run), run_attempt=1)
            if endpoint == "/actions/runs/42/attempts/1/jobs?per_page=100&page=1":
                job = dict(deepcopy(fixture.job), run_attempt=1)
                job["steps"][0]["conclusion"] = "skipped"
                return {"total_count": 1, "jobs": [job]}
            return original(endpoint)
        fixture.call = call
        self.run_boundary(fixture).upload.assert_called_once()
        self.assertIn("/actions/runs/42/attempts/1", fixture.requests)
        self.assertIn("/actions/runs/42/attempts/2", fixture.requests)

    def test_success_of_latest_attempt_cannot_authenticate_failed_certified_attempt(self):
        fixture = Fixture()
        fixture.attestations = [fixture.attestation(attempt=1)]
        original = fixture.call
        def call(endpoint):
            if endpoint == "/actions/runs/42/attempts/1":
                return dict(deepcopy(fixture.run), run_attempt=1, conclusion="failure")
            # The fixture's latest attempt 2 is successful, but is not the
            # certificate's attempt and must not authorize this manifest.
            return original(endpoint)
        fixture.call = call
        self.assert_rejected(fixture)

    def test_rerun_publish_can_reuse_earlier_build_artifact(self):
        fixture = Fixture()
        # No artifact upload-attempt constraint: the manifest authenticates the
        # package bytes even when only the publish job was rerun in attempt 2.
        fixture.job.pop("run_attempt")
        self.run_boundary(fixture).upload.assert_called_once()

    def test_duplicate_or_mismatched_publication_job_and_step_are_rejected(self):
        for change in ("duplicate job", "duplicate step", "wrong job run", "wrong job attempt", "wrong job source", "failed job"):
            with self.subTest(change=change):
                fixture = Fixture()
                if change == "duplicate job":
                    fixture.jobs.append(dict(deepcopy(fixture.job), id=8))
                elif change == "duplicate step":
                    fixture.job["steps"].append(deepcopy(fixture.job["steps"][0]))
                elif change == "wrong job run":
                    fixture.job["run_id"] = 999
                elif change == "wrong job attempt":
                    fixture.job["run_attempt"] = 1
                elif change == "wrong job source":
                    fixture.job["head_sha"] = SOURCE
                else:
                    fixture.job["conclusion"] = "failure"
                self.assert_rejected(fixture)

    def test_jobs_pagination_finds_late_publication_and_rejects_truncation(self):
        fixture = Fixture()
        fixture.jobs = [{"id": index + 100, "name": "other"} for index in range(100)] + [fixture.job]
        self.run_boundary(fixture).upload.assert_called_once()
        self.assertIn("/actions/runs/42/attempts/2/jobs?per_page=100&page=2", fixture.requests)
        fixture = Fixture()
        fixture.pages = [{"total_count": 2, "jobs": [fixture.job]}]
        self.assert_rejected(fixture)
        fixture = Fixture()
        fixture.pages = [{"total_count": 10001, "jobs": []}]
        self.assert_rejected(fixture)

    def test_signed_source_must_resolve_tag_and_be_contained_in_main(self):
        for field, value in (("tag_sha", WORKFLOW_SOURCE), ("compare_status", "diverged"), ("compare_status", "behind")):
            with self.subTest(field=field, value=value):
                fixture = Fixture()
                setattr(fixture, field, value)
                self.assert_rejected(fixture)

    def test_current_numeric_repository_identity_is_required(self):
        for changes in ({"id": 999}, {"id": True}, {"full_name": "owner/old-name"}):
            with self.subTest(changes=changes):
                fixture = Fixture()
                fixture.repository_data.update(changes)
                self.assert_rejected(fixture)

    def test_receipt_must_be_unique_verified_source_bound_and_well_formed(self):
        changes = [("sourceSha", WORKFLOW_SOURCE), ("verification", "not_verified"), ("schemaVersion", True),
                   ("binary", "../evil.msix"), ("binary", "path\\evil.msix"), ("binary", "evil.exe"), ("binary", None),
                   ("bytes", 0), ("bytes", -1), ("bytes", True), ("bytes", "12"), ("sha256", "bad"), ("sha256", None)]
        for key, value in changes:
            with self.subTest(field=key, value=value):
                fixture = Fixture()
                fixture.manifest["artifacts"][0][key] = value
                self.assert_rejected(fixture)
        for artifacts in ([], [None], [Fixture().manifest["artifacts"][0]] * 2):
            with self.subTest(artifacts=artifacts):
                fixture = Fixture()
                fixture.manifest["artifacts"] = artifacts
                self.assert_rejected(fixture)

    def test_downloaded_package_must_match_signed_size_and_hash(self):
        for payload in (PACKAGE + b"extra", b"X" * len(PACKAGE)):
            with self.subTest(payload=payload):
                fixture = Fixture()
                fixture.package = payload
                self.assert_rejected(fixture)

    def test_pending_commit_reproves_before_any_retry_mutation(self):
        fixture = Fixture()
        provider = fixture.provider(pending=True)
        provider.upload.side_effect = OSError("synthetic upload interruption")
        with self.assertRaises(OSError):
            self.run_boundary(fixture, provider)
        self.assertEqual(fixture.record["storeSubmissionId"], "S1")
        provider.reset_mock()
        fixture.verifier_error = subprocess.CalledProcessError(1, ["gh", "attestation", "verify"])
        with self.assertRaises(ValueError):
            self.run_boundary(fixture, provider)
        provider.call.assert_called_once_with("/submissions/S1/status")
        provider.upload.assert_not_called()

    def test_malformed_verifier_and_api_shapes_fail_closed_and_redacted(self):
        for result in ([], {}, [None], [{"verificationResult": None}], [{"verificationResult": {"signature": []}}]):
            with self.subTest(result=result):
                fixture = Fixture()
                fixture.attestations = result
                self.assert_rejected(fixture)
        fixture = Fixture()
        fixture.verifier_output = "untrusted verifier output"
        with self.assertRaisesRegex(ValueError, "^Store artifact provenance is missing or invalid$"):
            self.run_boundary(fixture)
        for target in ("repository_data", "workflow", "run"):
            with self.subTest(target=target):
                fixture = Fixture()
                setattr(fixture, target, [])
                self.assert_rejected(fixture)

    def test_wrong_verified_subject_digest_is_rejected(self):
        fixture = Fixture()
        fixture.attestations[0]["verificationResult"]["statement"]["subject"] = [{"digest": {"sha256": "0" * 64}}]
        self.assert_rejected(fixture)

    def test_polling_after_pending_commit_does_not_require_artifacts(self):
        fixture = Fixture()
        fixture.record.update(storeSubmissionId="S1", store="Certification")
        fixture.save_state(fixture.value, fixture.record)
        provider = fixture.provider()
        with patch.object(store.subprocess, "run") as command:
            store.resume(fixture, provider, fixture.release(123), Path("expired-artifact"))
        command.assert_not_called()
        provider.call.assert_called_once_with("/submissions/S1/status")
        provider.upload.assert_not_called()
        self.assertEqual(fixture.record["store"], "Published")

    def test_prerelease_is_not_eligible_for_store_mutation(self):
        fixture = Fixture()
        fixture.value["prerelease"] = True
        self.assert_rejected(fixture)


if __name__ == "__main__":
    unittest.main()
