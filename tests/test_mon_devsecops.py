"""Behavioral tests for security policy, documentation, and release boundaries.

All provider mutations are in-memory fakes. No signing, publication or Store
submission is performed by this suite.
"""
import base64
from contextlib import chdir
from copy import deepcopy
from datetime import date
import hashlib
import json
import os
import re
import shutil
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile
from urllib.parse import parse_qs, urlsplit

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import mon_ci_scope as scope
import mon_docs as docs
import mon_macos_release as mac
import mon_release as release
import mon_release_artifacts as artifacts
import mon_security as security
import mon_store_submission as store

SHA = "1" * 40


def versions(name):
    if name.endswith("Cargo.toml"):
        return '[package]\nversion = "1.2.3"\n'
    if name.endswith("package-lock.json"):
        return json.dumps({"packages": {"": {"version": "1.2.3"}}})
    return json.dumps({"version": "1.2.3"})


class Provider:
    def __init__(self):
        self.repository = "owner/repo"
        self.value = {"id": 123, "tag_name": "v1.2.3", "target_commitish": "main", "draft": True,
                      "prerelease": False, "name": "Release", "body": release.OPT_IN, "author": {"login": "owner"}, "assets": []}
        self.dispatches, self.uploads = [], []
        self.compare_status, self.commit = "identical", SHA

    def releases(self):
        return [deepcopy(self.value)]

    def release(self, _):
        return deepcopy(self.value)

    def file(self, name, _):
        return versions(name)

    def save_state(self, value, record, **changes):
        self.value = deepcopy(value)
        self.value.update(changes)
        encoded = base64.b64encode(json.dumps(record).encode()).decode()
        self.value["body"] = release.clean_body(value.get("body")) + f"\n\n<!-- mon-release:{encoded} -->"
        return deepcopy(self.value)

    def call(self, endpoint, data=None, method=None, binary=None):
        if "/permission" in endpoint:
            return {"permission": "admin"}
        if "/matching-refs/" in endpoint:
            return []
        if endpoint.startswith("/commits/"):
            return {"sha": self.commit}
        if endpoint.startswith("/compare/"):
            return {"status": self.compare_status}
        if endpoint.endswith("/dispatches"):
            self.dispatches.append(data)
            return None
        if "/assets?" in endpoint:
            self.uploads.append(endpoint)
            name = parse_qs(urlsplit(endpoint).query)["name"][0]
            self.value["assets"].append({"name": name, "size": len(binary), "digest": "sha256:" + hashlib.sha256(binary).hexdigest()})
            return {"id": len(self.uploads)}
        raise AssertionError(endpoint)


class ReleaseWorkflowTests(unittest.TestCase):
    def test_checkout_source_selects_the_admitted_older_main_commit_in_a_real_repository(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def git(*args):
                return subprocess.run(['git', *args], cwd=root, check=True, capture_output=True,
                                      text=True, encoding='utf8').stdout.strip()
            git('init', '-q', '-b', 'main')
            source = root / 'source.txt'
            source.write_text('admitted version', encoding='utf8')
            git('add', '.')
            git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-qm', 'admitted')
            admitted = git('rev-parse', 'HEAD')
            source.write_text('new main version', encoding='utf8')
            git('add', '.')
            git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-qm', 'main advanced')
            git('update-ref', 'refs/remotes/origin/main', git('rev-parse', 'HEAD'))
            with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main'}), patch.object(release, 'ROOT', root):
                release.checkout_source(admitted)
            self.assertEqual(git('rev-parse', 'HEAD'), admitted)
            self.assertEqual(source.read_text(encoding='utf8'), 'admitted version')

    def test_release_builds_bootstrap_source_admission_from_main(self):
        workflow = yaml.safe_load((release.ROOT / '.github/workflows/release-build.yml').read_text(encoding='utf8'))
        for name in ('macos-prepare', 'macos-sign', 'windows', 'store', 'publish'):
            steps = workflow['jobs'][name]['steps']
            with self.subTest(job=name):
                self.assertEqual(steps[0]['with']['ref'], 'main')
                self.assertEqual(steps[0]['with']['fetch-depth'], 0)
                admission = next(s for s in steps if s.get('name') == 'Check out admitted source with trusted main code')
                self.assertEqual(admission['env']['MON_SOURCE_SHA'], '${{ needs.guard.outputs.source_sha }}')
                self.assertIn('mon_release.py checkout', admission['run'])
                self.assertLess(next(i for i,s in enumerate(steps) if s.get('name') == admission['name']),
                                next(i for i,s in enumerate(steps) if 'run' in s and 'mon_release.py checkout' not in s['run']))

    def test_checkout_source_rejects_untrusted_refs_dirty_trees_and_non_main_commits(self):
        for values in ({'GITHUB_REF': 'refs/heads/other'}, {'GITHUB_REF': 'refs/heads/main'}):
            with patch.dict(os.environ, values), patch.object(release.subprocess, 'run') as run:
                with self.assertRaises(ValueError):
                    release.checkout_source(SHA if values['GITHUB_REF'] != 'refs/heads/main' else '--option')
                run.assert_not_called()
        for dirty, contained in ((True, True), (False, False), (False, True)):
            calls = []
            def run(args, **kwargs):
                calls.append(args)
                return subprocess.CompletedProcess(args, 0 if args[1] != 'merge-base' or contained else 1,
                                                   ' M unreviewed.py' if args[1] == 'status' and dirty else '')
            with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main'}), patch.object(release.subprocess, 'run', side_effect=run):
                if dirty or not contained:
                    with self.assertRaises(ValueError): release.checkout_source(SHA)
                    self.assertFalse(any(args[1] == 'checkout' for args in calls))
                else:
                    release.checkout_source(SHA)
                    self.assertEqual(calls[-1], ['git', 'checkout', '--detach', SHA])

    def test_guard_can_read_drafts_without_giving_build_jobs_write_access(self):
        workflow = yaml.safe_load((release.ROOT / ".github/workflows/release-build.yml").read_text())
        guard = workflow["jobs"]["guard"]
        permissions = guard.get("permissions", workflow["permissions"])
        api = Provider()
        read_release = api.release

        def read_draft(release_id):
            if permissions.get("contents") != "write":
                raise ValueError("GitHub request failed (404): draft requires push access")
            return read_release(release_id)

        step = next(step for step in guard["steps"] if step.get("id") == "guard")
        script = "\n".join(step["run"].splitlines()[1:-1])
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
            "GITHUB_REF": "refs/heads/main", "GITHUB_RUN_ID": "42", "GITHUB_OUTPUT": str(Path(temporary) / "output"),
            "MON_RELEASE_ID": "123", "MON_SOURCE_SHA": SHA, "MON_PRIVATE": "false", "MON_ENABLED": "true",
            "MON_PUBLISH_ENABLED": "false", "MON_DRY_RUN": "true",
        }):
            release.discover(api)
            with patch.object(api, "release", side_effect=read_draft), patch.object(release, "GitHub", return_value=api):
                exec(compile(script, "release-guard", "exec"), {})
            self.assertEqual(Path(os.environ["GITHUB_OUTPUT"]).read_text(), "tag=v1.2.3\nsource_sha=" + SHA + "\n")
        for name in ("ci", "security", "macos-prepare", "macos-sign", "windows", "store"):
            with self.subTest(job=name):
                effective = workflow["jobs"][name].get("permissions", workflow["permissions"])
                self.assertEqual(effective.get("contents"), "read")


class DesktopWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = yaml.safe_load((release.ROOT / ".github/workflows/desktop-platform-build.yml").read_text())
        self.events = self.workflow.get("on", self.workflow.get(True))

    @staticmethod
    def expression(value, context):
        """Evaluate the small boolean subset used by the native step guards."""
        value = value.removeprefix("${{ ").removesuffix(" }}")
        value = re.sub(r"\b(?:inputs|github|runner|env|steps|matrix|needs)(?:\.[A-Za-z_][A-Za-z_0-9-]*)+",
                       lambda match: repr(context[match[0]]), value)
        value = value.replace("&&", " and ").replace("||", " or ")
        value = re.sub(r"!(?!=)", " not ", value).strip()
        return eval(value, {"__builtins__": {}}, {"always": lambda: True,
                    "success": lambda: context.get("success()", True),
                    "cancelled": lambda: context.get("cancelled()", False)})

    def native_steps(self, event="pull_request", full=False, suite="", operating_system="Windows", rust_cache_hit=False,
                     skip_windows_msix=False, workflow="Desktop platform validation"):
        context = {"inputs.full": full, "inputs.suite": suite, "github.event_name": event,
                   "inputs.skip_windows_msix": skip_windows_msix,
                   "runner.os": operating_system, "github.ref": "refs/heads/main",
                   "github.workflow": workflow, "inputs.source_ref": "", "github.sha": SHA,
                   "steps.native_downloads.outputs.cache-hit": "false",
                   **{f"steps.{name}_cache.outputs.cache-hit": "false" for name in (
                       "windows_runtime", "windows_capture", "macos_capture", "macos_workers")},
                   "steps.rust_cache.outputs.cache-hit": str(rust_cache_hit).lower()}
        native = self.workflow["jobs"]["native"]
        context["env.MON_CI_FULL"] = str(self.expression(native["env"]["MON_CI_FULL"], context)).lower()
        context["env.MON_SAVE_CI_CACHE"] = str(self.expression(native["env"]["MON_SAVE_CI_CACHE"], context)).lower()
        context["env.MON_SKIP_WINDOWS_MSIX"] = str(skip_windows_msix).lower()
        return [step for step in native["steps"] if self.expression(step.get("if", "True"), context)]

    def native_commands(self, event="pull_request", full=False, suite="", operating_system="Windows"):
        return "\n".join(step.get("run", "") for step in self.native_steps(event, full, suite, operating_system))

    def test_native_cache_is_exact_and_only_successful_trusted_main_jobs_save(self):
        context = {"github.ref": "refs/heads/main", "github.workflow": "Desktop platform validation",
                   "github.event_name": "push", "inputs.source_ref": "", "github.sha": SHA}
        expression = self.workflow['jobs']['native']['env']['MON_SAVE_CI_CACHE']
        self.assertTrue(self.expression(expression, context))
        for change in ({'github.ref': 'refs/pull/1/merge'}, {'github.event_name': 'pull_request'},
                       {'inputs.source_ref': '2' * 40}, {'github.workflow': 'Release build'}):
            self.assertFalse(self.expression(expression, {**context, **change}), change)
        self.assertTrue(self.expression(expression, {**context, 'github.event_name': 'schedule', 'inputs.source_ref': SHA}))
        for job in ('native', 'browser'):
            for step in self.workflow['jobs'][job]['steps']:
                action = step.get('uses', '')
                if action.startswith('actions/cache/restore@'):
                    self.assertNotIn('restore-keys', step['with'])
                if action.startswith('actions/cache/save@'):
                    self.assertIn('success()', step['if'])
                    self.assertIn("env.MON_SAVE_CI_CACHE == 'true'", step['if'])

    def test_full_rust_cache_preserves_legacy_fallback_without_duplicate_warm_restores(self):
        for full, hit, count in ((False, False, 1), (False, True, 1), (True, False, 2), (True, True, 1)):
            with self.subTest(full=full, hit=hit):
                steps = self.native_steps(event='workflow_dispatch', suite='full' if full else 'quick', rust_cache_hit=hit)
                caches = [s for s in steps if s.get('uses', '').startswith('Swatinem/rust-cache@')]
                self.assertEqual(len(caches), count)
                primary = next(s for s in caches if s.get('id') == 'rust_cache')
                self.assertEqual(self.expression(primary['with']['key'], {'env.MON_CI_FULL': str(full).lower()}), 'full' if full else '')
                self.assertEqual(primary['with']['workspaces'], '. -> app/src-tauri/target')
                if count == 2:
                    fallback = next(s for s in caches if s is not primary)
                    self.assertNotIn('key', fallback['with'])
                    self.assertEqual(fallback['with']['save-if'], 'false')

    def test_native_intermediate_caches_have_component_specific_exact_inputs_and_paths(self):
        native = self.workflow['jobs']['native']['steps']
        for component, operating_system in (("windows-runtime", "Windows"), ("windows-capture", "Windows"),
                                            ("macos-capture", "macOS"), ("macos-workers", "macOS")):
            identifier = component.replace('-', '_')
            fingerprint = next(s for s in native if s.get('id') == identifier + '_key')
            self.assertIn('--component ' + component, fingerprint['run'])
            self.assertIn("runner.os == '" + operating_system + "'", fingerprint['if'])
            restore = next(s for s in native if s.get('id') == identifier + '_cache')
            save = next(s for s in native if s.get('name') == 'Save ' + component + ' build intermediates')
            self.assertEqual(restore['with'], save['with'])
            self.assertNotIn('restore-keys', restore['with'])
            self.assertIn(identifier + '_key.outputs.paths', restore['with']['path'])
            self.assertIn("env.MON_SAVE_CI_CACHE == 'true'", save['if'])

    def test_windows_resources_and_all_native_checks_precede_both_package_flavors(self):
        steps = self.native_steps(event='workflow_call', full=True)
        names = [s.get('name') for s in steps]
        prepare = names.index('Prepare Windows resources before native validation')
        native = names.index('Validate Windows capture and model catalog')
        llm = names.index('Validate upgraded LLM runtimes with representative weights')
        qa = names.index('Build Windows QA installer')
        qa_dpi = names.index('Verify DPI declarations in the linked Windows QA executable')
        store = names.index('Build and verify Microsoft Store MSIX')
        store_dpi = names.index('Verify DPI declarations in the linked Windows Store executable')
        self.assertLess(prepare, native)
        self.assertLess(native, llm)
        self.assertLess(llm, qa)
        self.assertLess(qa, qa_dpi)
        self.assertLess(qa_dpi, store)
        self.assertLess(store, store_dpi)
        preparation = steps[prepare]
        self.assertIn('-Receipt $receipt', preparation['run'])
        self.assertIn('MON_WINDOWS_PREPARATION_RECEIPT=$receipt', preparation['run'])
        for index in (prepare, qa, store):
            self.assertIn('mon_windows_preparation.py run --phase', steps[index]['run'])

    def test_release_delegates_only_msix_to_its_required_store_job(self):
        self.assertFalse(self.events['workflow_call']['inputs']['skip_windows_msix']['default'])
        steps = self.native_steps(event='workflow_dispatch', full=True, skip_windows_msix=True, workflow='Release build')
        commands = '\n'.join(s.get('run', '') for s in steps)
        self.assertIn('tauri:windows:qa:build', commands)
        self.assertIn('--suite full --all-targets', commands)
        self.assertIn('mon_llama_runtime_check.py', commands)
        self.assertNotIn('tauri:windows:store:build', commands)
        uploads = [s['with']['name'] for s in steps if s.get('uses', '').startswith('actions/upload-artifact@')]
        self.assertIn('mon-windows-x64-qa-identity', uploads)
        self.assertNotIn('minuteson-windows-x64-store-identity', uploads)
        workflow = yaml.safe_load((release.ROOT / '.github/workflows/release-build.yml').read_text())
        self.assertTrue(workflow['jobs']['ci']['with']['skip_windows_msix'])
        store = workflow['jobs']['store']
        store_commands = '\n'.join(s.get('run', '') for s in store['steps'])
        self.assertEqual(store_commands.count('tauri:windows:store:build'), 1)
        self.assertIn('MON_WINDOWS_DPI_BINARY', store_commands)
        self.assertIn('test_mon_windows_dpi.py', store_commands)
        self.assertIn('ci', store['needs'])
        self.assertIn('store', workflow['jobs']['publish']['needs'])

    def test_store_delegation_is_rejected_outside_full_release_validation(self):
        step = next(s for s in self.workflow['jobs']['changes']['steps'] if s.get('name') == 'Validate delegated Microsoft Store check')
        script = '\n'.join(step['run'].splitlines()[1:-1])
        for skip, full, caller in (('true', 'false', 'Release build'), ('true', 'true', 'Desktop platform validation'),
                                    ('true', 'true', 'Release build'), ('false', 'false', 'Desktop platform validation')):
            with self.subTest(skip=skip, full=full, caller=caller), patch.dict(os.environ, {
                    'MON_SKIP_WINDOWS_MSIX': skip, 'MON_CI_FULL': full, 'MON_CALLER': caller}):
                if skip == 'true' and (full != 'true' or caller != 'Release build'):
                    with self.assertRaises(SystemExit):
                        exec(compile(script, 'store-delegation', 'exec'), {})
                else:
                    exec(compile(script, 'store-delegation', 'exec'), {})

    def test_swift_analysis_cache_preserves_extraction_and_trusted_main_writes(self):
        workflow = yaml.safe_load((release.ROOT / '.github/workflows/security.yml').read_text())
        job = workflow['jobs']['codeql']
        cache = next(step for step in job['steps'] if step.get('uses', '').startswith('Swatinem/rust-cache@'))
        self.assertEqual(cache['if'], "matrix.language == 'swift'")
        self.assertEqual(cache['with']['workspaces'], '. -> probes/macos-capture/.build/capture-core')
        self.assertEqual(cache['with']['save-if'], "${{ env.MON_SAVE_CI_CACHE == 'true' }}")
        context = {'github.ref': 'refs/heads/main', 'github.workflow': 'Security validation',
                   'github.event_name': 'push', 'inputs.source_ref': '', 'github.sha': SHA}
        expression = job['env']['MON_SAVE_CI_CACHE']
        self.assertTrue(self.expression(expression, context))
        for change in ({'github.event_name': 'pull_request'}, {'inputs.source_ref': '2' * 40},
                       {'github.workflow': 'Release build'}, {'github.ref': 'refs/heads/topic'}):
            self.assertFalse(self.expression(expression, {**context, **change}), change)
        extraction = next(step for step in job['steps'] if step.get('name') == 'Swift extraction build')
        self.assertEqual(extraction['if'], "matrix.language == 'swift'")
        self.assertIn('swift build', extraction['run'])
        self.assertNotIn('cache-hit', extraction['if'])

    @unittest.skipUnless(os.name != 'nt' and shutil.which('bash'), 'Run the CI Bash control test on Unix, not the Windows WSL launcher')
    def test_required_browser_check_failure_cannot_be_masked_by_a_later_success(self):
        cases = (
            ('Check generated command contracts', 'python tools/mon_contracts.py --check', 'python tools/mon_model_catalog.py --check'),
            ('Frontend tests and build', 'npm test', 'npm run build'),
        )
        for name, check, later in cases:
            with self.subTest(step=name):
                step = next(s for s in self.workflow['jobs']['browser']['steps'] if s.get('name') == name)
                self.assertEqual(step['shell'], 'bash')
                script = step['run'].replace(check, 'false').replace(later, "printf 'incorrectly continued'")
                result = subprocess.run(['bash', '-c', script], capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('incorrectly continued', result.stdout)

    def test_pr_and_main_run_quick_without_native_resource_builds(self):
        for event in ("pull_request", "push"):
            for operating_system in ("Windows", "macOS"):
                with self.subTest(event=event, os=operating_system):
                    commands = self.native_commands(event=event, operating_system=operating_system)
                    self.assertIn("--suite quick --all-targets --processes 2 --threads 4", commands)
                    self.assertIn("--timings-json", commands)
                    for excluded in ("tauri:windows:qa:build", "tauri:windows:store:build", "--swift-fixture",
                                     "build-core.sh", "swift build", "ctest --test-dir", "--suite full"):
                        self.assertNotIn(excluded, commands)

    def test_reusable_full_and_manual_default_keep_packaging_and_native_checks(self):
        dispatch = self.events["workflow_dispatch"]["inputs"]["suite"]
        self.assertEqual(dispatch["default"], "full")
        self.assertEqual(set(dispatch["options"]), {"quick", "full"})
        for event, full, suite in (("workflow_call", True, ""), ("workflow_dispatch", False, "full")):
            windows = self.native_commands(event, full, suite, "Windows")
            self.assertIn("tauri:windows:qa:build", windows)
            self.assertIn("tauri:windows:store:build", windows)
            self.assertIn("ctest --test-dir", windows)
            self.assertIn("test_mon_windows_dpi.py", windows)
            self.assertIn("--suite full --all-targets", windows)
            macos = self.native_commands(event, full, suite, "macOS")
            self.assertIn("build-core.sh", macos)
            self.assertIn("swift test", macos)
            self.assertIn("--suite full --all-targets --swift-fixture", macos)
            self.assertNotIn("--suite quick", windows + macos)
            self.assertNotIn("--processes", windows + macos)
            self.assertNotIn("--threads", windows + macos)
        self.assertIn("--suite quick", self.native_commands("workflow_dispatch", False, "quick"))

    def test_validation_uploads_are_limited_to_diagnostics_and_json_build_identity(self):
        allowed_paths = {
            "output/playwright/",
            "output/rust-tests/",
            "output/windows-store-work/*/source.json",
            "output/windows-store-work/*/build.json",
            "output/windows-qa-work/*-build.json",
            "output/rust-tests/benchmark-${{ runner.os }}.json",
            "output/rust-tests/rust-audit-${{ runner.os }}.json",
            "output/rust-tests/cargo-timings/**/*.html",
        }
        for job, definition in self.workflow["jobs"].items():
            for step in definition.get("steps", []):
                if step.get("uses", "").startswith("actions/upload-artifact@"):
                    with self.subTest(job=job, step=step.get("name")):
                        paths = set(step["with"]["path"].splitlines())
                        self.assertTrue(paths)
                        self.assertLessEqual(paths, allowed_paths)

    def test_quick_and_full_keep_diagnostics_without_uploading_installers(self):
        for event, full, suite in (("pull_request", False, ""), ("push", False, ""),
                                   ("workflow_dispatch", False, "quick"),
                                   ("workflow_dispatch", False, "full"), ("workflow_call", True, "")):
            for operating_system in ("Windows", "macOS"):
                with self.subTest(event=event, full=full, suite=suite, os=operating_system):
                    uploads = {step["with"]["name"] for step in self.native_steps(event, full, suite, operating_system)
                               if step.get("uses", "").startswith("actions/upload-artifact@")}
                    expected = {"rust-tests-${{ runner.os }}"}
                    if operating_system == "Windows" and (full or suite == "full"):
                        expected |= {"minuteson-windows-x64-store-identity", "mon-windows-x64-qa-identity"}
                    self.assertEqual(uploads, expected)

    def test_daily_and_release_call_full_at_the_selected_source(self):
        daily = yaml.safe_load((release.ROOT / ".github/workflows/desktop-daily.yml").read_text())
        events = daily.get("on", daily.get(True))
        self.assertEqual(events["schedule"], [{"cron": "17 19 * * *"}])
        self.assertEqual(set(events), {"schedule", "workflow_dispatch"})
        job = daily["jobs"]["validate"]
        self.assertEqual(job["uses"], "./.github/workflows/desktop-platform-build.yml")
        self.assertEqual(job["with"], {"source_ref": "${{ github.sha }}", "full": True})
        self.assertFalse(daily["concurrency"]["cancel-in-progress"])
        self.assertEqual(daily["permissions"], {"contents": "read"})
        release_workflow = yaml.safe_load((release.ROOT / ".github/workflows/release-build.yml").read_text())
        release_ci = release_workflow["jobs"]["ci"]
        self.assertTrue(release_ci["with"]["full"])
        self.assertEqual(release_ci["with"]["source_ref"], "${{ needs.guard.outputs.source_sha }}")
        self.assertNotEqual(daily["concurrency"]["group"], self.workflow["concurrency"]["group"])

    def test_gate_rejects_required_failure_cancellation_or_skip_but_allows_docs_only(self):
        gate = self.workflow["jobs"]["gate"]
        self.assertEqual(gate["if"], "always() && (inputs.benchmark_mode == '' || inputs.benchmark_mode == 'none')")
        self.assertEqual(set(gate["needs"]), {"changes", "python", "browser", "native"})
        script = "\n".join(gate["steps"][0]["run"].splitlines()[1:-1])
        successful = {"MON_CHANGES": "success", "MON_PYTHON": "success", "MON_BROWSER": "success",
                      "MON_NATIVE": "success", "MON_REQUIRED": "true"}
        with patch.dict(os.environ, successful):
            exec(compile(script, "ci-gate", "exec"), {})
        for name in ("MON_CHANGES", "MON_PYTHON", "MON_BROWSER", "MON_NATIVE"):
            for result in ("failure", "cancelled", "skipped"):
                with self.subTest(job=name, result=result), patch.dict(os.environ, {**successful, name: result}):
                    with self.assertRaises(SystemExit):
                        exec(compile(script, "ci-gate", "exec"), {})
        with patch.dict(os.environ, {**successful, "MON_REQUIRED": "false", "MON_BROWSER": "skipped", "MON_NATIVE": "skipped"}):
            exec(compile(script, "ci-gate", "exec"), {})

    def test_docs_scope_and_full_scope_run_through_the_actual_scope_program(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "output"
            with patch.dict(os.environ, {"MON_BASE": SHA, "MON_FULL": "false", "GITHUB_OUTPUT": str(output)}), \
                    patch.object(scope.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"docs/guides/testing.md\0README.md\0")):
                scope.main()
            self.assertEqual(output.read_text(), "native=false\n")
            output.unlink()
            with patch.dict(os.environ, {"MON_BASE": SHA, "MON_FULL": "true", "GITHUB_OUTPUT": str(output)}), \
                    patch.object(scope.subprocess, "run") as diff:
                scope.main()
                diff.assert_not_called()
            self.assertEqual(output.read_text(), "native=true\n")

    def test_manual_benchmarks_are_separate_from_normal_validation(self):
        dispatch = self.events['workflow_dispatch']['inputs']
        self.assertEqual(dispatch['benchmark_mode']['default'], 'none')
        self.assertEqual(dispatch['benchmark_mode']['options'], ['none', 'cache', 'rust', 'sequence'])
        self.assertIn('effective Rust settings audit', dispatch['benchmark_mode']['description'])
        benchmark = self.workflow['jobs']['benchmark']
        self.assertEqual(benchmark['strategy']['matrix']['os'], ['macos-15', 'windows-2022'])
        self.assertFalse(benchmark['strategy']['fail-fast'])
        self.assertEqual(benchmark['timeout-minutes'], 180)
        for mode in ('', 'none', 'cache', 'rust', 'sequence'):
            context = {'inputs.benchmark_mode': mode, 'github.event_name': 'workflow_dispatch',
                       'needs.benchmark_admission.result': 'success',
                       'needs.full_validation.result': 'success' if mode == 'sequence' else 'skipped'}
            ordinary = mode in ('', 'none')
            for name in ('changes', 'python', 'gate'):
                with self.subTest(mode=mode, job=name):
                    self.assertEqual(bool(self.expression(self.workflow['jobs'][name]['if'], context)), ordinary)
            self.assertEqual(bool(self.expression(benchmark['if'], context)), not ordinary)
        self.assertFalse(self.expression(benchmark['if'], {**context, 'inputs.benchmark_mode': 'cache', 'github.event_name': 'pull_request'}))
        for name in ('native', 'browser'):
            self.assertEqual(self.workflow['jobs'][name]['needs'], 'changes')

    def test_benchmark_preserves_harness_before_checking_out_pinned_source(self):
        steps = self.workflow['jobs']['benchmark']['steps']
        checkouts = [(index, step) for index, step in enumerate(steps) if step.get('uses', '').startswith('actions/checkout@')]
        self.assertEqual(len(checkouts), 2)
        self.assertEqual(checkouts[0][1]['with']['ref'], '${{ github.sha }}')
        self.assertEqual(checkouts[1][1]['with']['ref'], '${{ needs.benchmark_admission.outputs.source_sha }}')
        self.assertTrue(all(step['with']['persist-credentials'] is False for _, step in checkouts))
        preserve = next(index for index, step in enumerate(steps) if step.get('name') == 'Validate inputs and preserve standalone harness')
        self.assertLess(checkouts[0][0], preserve)
        self.assertLess(preserve, checkouts[1][0])
        measurement = next(step for step in steps if step.get('name') == 'Measure native caches or audit effective Rust settings at the pinned source')
        self.assertIn('python "$MON_CI_BENCHMARK_TOOL"', measurement['run'])
        for argument in ('--root "$GITHUB_WORKSPACE"', '--source-sha "$MON_BENCHMARK_SOURCE_SHA"', '--mode "$MON_BENCHMARK_MODE"'):
            self.assertIn(argument, measurement['run'])

    def test_benchmark_admission_requires_full_sha_and_copies_tool_outside_checkout(self):
        step = next(s for s in self.workflow['jobs']['benchmark']['steps'] if s.get('name') == 'Validate inputs and preserve standalone harness')
        script = '\n'.join(step['run'].splitlines()[1:-1])
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary).resolve()
            root, runner = directory / 'workspace', directory / 'runner'
            (root / 'tools').mkdir(parents=True)
            runner.mkdir()
            source = root / 'tools/mon_ci_benchmark.py'
            source.write_bytes(b'# standalone harness fixture; never executed\n')
            environment_file = directory / 'github-env'
            with chdir(root), patch.dict(os.environ, {'RUNNER_TEMP': str(runner), 'GITHUB_ENV': str(environment_file)}):
                for mode, sha in (('cache', ''), ('rust', '1234567'), ('cache', 'g' * 40), ('invalid', SHA)):
                    with self.subTest(mode=mode, sha=sha), patch.dict(os.environ, {'MON_BENCHMARK_MODE': mode, 'MON_BENCHMARK_SOURCE_SHA': sha}):
                        with self.assertRaises(SystemExit):
                            exec(compile(script, 'benchmark-admission', 'exec'), {})
                    self.assertEqual(list(runner.iterdir()), [])
                with patch.dict(os.environ, {'MON_BENCHMARK_MODE': 'cache', 'MON_BENCHMARK_SOURCE_SHA': SHA}):
                    exec(compile(script, 'benchmark-admission', 'exec'), {})
            copied = runner / 'mon_ci_benchmark.py'
            self.assertEqual(copied.read_bytes(), source.read_bytes())
            self.assertEqual(environment_file.read_text(), 'MON_CI_BENCHMARK_TOOL=' + str(copied) + '\n')

    def test_benchmark_source_is_admitted_before_the_frozen_reusable_workflow_can_run(self):
        jobs = self.workflow['jobs']
        admission = jobs['benchmark_admission']
        self.assertEqual(admission['outputs'], {
            'source_sha': '${{ steps.validate.outputs.source_sha }}',
            'effective_mode': '${{ steps.validate.outputs.effective_mode }}',
        })
        step = next(s for s in admission['steps'] if s.get('id') == 'validate')
        script = '\n'.join(step['run'].splitlines()[1:-1])
        frozen = '86952a9e04e8d4d6b5ae1b0f7844e889ff5006cb'
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'outputs'
            for mode, source in [('cache', ''), ('rust', 'abc1234'), ('cache', 'g' * 40),
                                 ('sequence', SHA), ('invalid', frozen)]:
                with self.subTest(mode=mode, source=source), patch.dict(os.environ, {
                    'MON_REQUESTED_MODE': mode, 'MON_REQUESTED_SOURCE_SHA': source, 'GITHUB_OUTPUT': str(output),
                }):
                    with self.assertRaises(SystemExit):
                        exec(compile(script, 'benchmark-admission', 'exec'), {})
                    self.assertFalse(output.exists())
            for mode, source, effective in [('cache', SHA, 'cache'), ('rust', SHA, 'rust'), ('sequence', frozen, 'cache')]:
                with self.subTest(mode=mode), patch.dict(os.environ, {
                    'MON_REQUESTED_MODE': mode, 'MON_REQUESTED_SOURCE_SHA': source, 'GITHUB_OUTPUT': str(output),
                }):
                    exec(compile(script, 'benchmark-admission', 'exec'), {})
                    self.assertEqual(output.read_text(), f'source_sha={source}\neffective_mode={effective}\n')
                    output.unlink()
        full = jobs['full_validation']
        self.assertEqual(full['needs'], 'benchmark_admission')
        self.assertEqual(full['uses'], f's-kodani/minuteson/.github/workflows/desktop-platform-build.yml@{frozen}')
        self.assertEqual(full['with'], {'source_ref': '${{ needs.benchmark_admission.outputs.source_sha }}', 'full': True})
        self.assertNotIn('secrets', full)
        self.assertNotIn('permissions', full)
        self.assertEqual(self.workflow['permissions'], {'contents': 'read'})
        for mode, result, allowed in [('sequence', 'success', True), ('sequence', 'failure', False),
                                      ('cache', 'success', False), ('rust', 'success', False)]:
            self.assertEqual(bool(self.expression(full['if'], {
                'inputs.benchmark_mode': mode, 'needs.benchmark_admission.result': result,
            })), allowed)

    def test_sequence_waits_for_full_success_and_direct_modes_allow_only_a_skipped_full(self):
        benchmark = self.workflow['jobs']['benchmark']
        self.assertEqual(set(benchmark['needs']), {'benchmark_admission', 'full_validation'})
        context = {'github.event_name': 'workflow_dispatch'}
        for mode in ('cache', 'rust', 'sequence'):
            for admission in ('success', 'failure', 'cancelled', 'skipped'):
                for full in ('success', 'failure', 'cancelled', 'skipped'):
                    for cancelled in (False, True):
                        expected = not cancelled and admission == 'success' and full == ('success' if mode == 'sequence' else 'skipped')
                        with self.subTest(mode=mode, admission=admission, full=full, cancelled=cancelled):
                            self.assertEqual(bool(self.expression(benchmark['if'], {
                                **context, 'inputs.benchmark_mode': mode, 'needs.benchmark_admission.result': admission,
                                'needs.full_validation.result': full, 'cancelled()': cancelled,
                            })), expected)
        self.assertEqual(benchmark['env']['MON_BENCHMARK_MODE'], '${{ needs.benchmark_admission.outputs.effective_mode }}')
        self.assertEqual(benchmark['env']['MON_BENCHMARK_SOURCE_SHA'], '${{ needs.benchmark_admission.outputs.source_sha }}')
        steps = benchmark['steps']
        measurement = next(i for i, s in enumerate(steps) if s.get('name') == 'Measure native caches or audit effective Rust settings at the pinned source')
        audit = next(i for i, s in enumerate(steps) if s.get('name') == 'Audit effective Rust settings after successful cache measurements')
        upload = next(i for i, s in enumerate(steps) if s.get('uses', '').startswith('actions/upload-artifact@'))
        self.assertLess(measurement, audit)
        self.assertLess(audit, upload)
        self.assertIn('--mode rust --report output/rust-tests/rust-audit-${{ runner.os }}.json', steps[audit]['run'])
        for mode, succeeded, expected in [('sequence', True, True), ('sequence', False, False), ('cache', True, False), ('rust', True, False)]:
            self.assertEqual(bool(self.expression(steps[audit]['if'], {'inputs.benchmark_mode': mode, 'success()': succeeded})), expected)

    def test_benchmark_job_summary_preserves_source_and_harness_provenance_without_raw_errors(self):
        step = next(s for s in self.workflow['jobs']['benchmark']['steps'] if s.get('name') == 'Summarize benchmark reports')
        self.assertEqual(step['if'], 'always()')
        script = '\n'.join(step['run'].splitlines()[1:-1])
        with tempfile.TemporaryDirectory() as temporary, chdir(temporary):
            reports = Path('output/rust-tests')
            reports.mkdir(parents=True)
            summary = Path(temporary) / 'summary'
            common = {'sourceSha': SHA, 'workflowSha': '2' * 40, 'completed': True,
                      'sourceUnchanged': True, 'exitCode': 0, 'totalSeconds': 123.4567}
            (reports / 'benchmark-Windows.json').write_text(json.dumps({**common, 'mode': 'cache',
                'error': 'never-copy-private-output', 'cache': {'actionsCacheHit': None,
                'trials': [{'status': 'success', 'totalSeconds': 42.1234}, {'status': 'running', 'totalSeconds': None}],
                'summary': {'coldSeconds': 42.1234, 'warmMedianSeconds': 10.5}}}))
            (reports / 'rust-audit-Windows.json').write_text(json.dumps({**common, 'mode': 'rust', 'rust': {'status': 'redundant'}}))
            with patch.dict(os.environ, {'RUNNER_OS': 'Windows', 'GITHUB_STEP_SUMMARY': str(summary)}):
                exec(compile(script, 'benchmark-summary', 'exec'), {})
            text = summary.read_text()
            for expected in (SHA, '2' * 40, '42.123', '10.5', 'redundant', 'running', 'actualActionsCacheHit=null', 'retained fixed downloads'):
                self.assertIn(expected, text)
            self.assertEqual(text.count('Measurement source SHA'), 2)
            self.assertNotIn('never-copy-private-output', text)
            self.assertNotIn('None', text)
            for path in reports.iterdir():
                path.unlink()
            summary.unlink()
            with patch.dict(os.environ, {'RUNNER_OS': 'Windows', 'GITHUB_STEP_SUMMARY': str(summary)}):
                exec(compile(script, 'benchmark-summary', 'exec'), {})
            self.assertFalse(summary.exists())

    def test_benchmark_never_saves_native_or_dependency_caches_and_uploads_only_reports(self):
        benchmark = self.workflow['jobs']['benchmark']
        self.assertEqual(benchmark['env']['MON_SAVE_CI_CACHE'], 'false')
        for step in benchmark['steps']:
            action = step.get('uses', '')
            self.assertFalse(action.startswith('actions/cache/'))
            if action.startswith('Swatinem/rust-cache@'):
                self.assertEqual(step['with']['save-if'], 'false')
            if action.startswith(('actions/setup-node@', 'actions/setup-python@')):
                self.assertNotIn('cache', step['with'])
            if action.startswith('actions/upload-artifact@'):
                self.assertEqual(step['if'], 'always()')
                self.assertEqual(set(step['with']['path'].splitlines()), {
                    'output/rust-tests/benchmark-${{ runner.os }}.json',
                    'output/rust-tests/rust-audit-${{ runner.os }}.json',
                    'output/rust-tests/cargo-timings/**/*.html',
                })
        frontend = next(step for step in benchmark['steps'] if step.get('name') == 'Install pinned frontend dependencies')
        self.assertEqual(frontend['if'], "env.MON_BENCHMARK_MODE == 'cache'")


class SecurityPolicyTests(unittest.TestCase):
    def test_severity_boundary_unknown_and_exact_alias_exception(self):
        findings = [{"tool": "osv", "id": "OSV-test", "aliases": ["CVE-test"], "target": "npm:example@1", "score": value}
                    for value in (6.9, 7.0, 9.8, None)]
        self.assertEqual(len(security.blocked(findings, [])), 3)
        exception = {"tool": "osv", "id": "CVE-test", "target": "npm:example@1"}
        self.assertEqual(security.blocked(findings, [exception]), [])
        self.assertEqual(len(security.blocked(findings, [{**exception, "target": "npm:example@2"}])), 3)

    def test_cvss_and_provider_severity_are_not_downgraded(self):
        advisory = {"severity": [{"type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"}], "database_specific": {"severity": "LOW"}}
        self.assertEqual(security.score(advisory), 9.8)
        self.assertEqual(security.score({"database_specific": {"severity": "HIGH"}}), 7)
        self.assertIsNone(security.score({}))

    def test_aliases_share_score_and_maintenance_notices_remain_nonblocking(self):
        advisories = [{"id": "RUSTSEC-test", "aliases": ["GHSA-test"]},
                      {"id": "GHSA-test", "aliases": ["RUSTSEC-test"], "database_specific": {"severity": "MODERATE"}},
                      {"id": "unmaintained", "affected": [{"database_specific": {"informational": "unmaintained"}}]},
                      {"id": "unsound", "affected": [{"database_specific": {"informational": "unsound"}}]}]
        findings = security.osv_findings({"results": [{"packages": [{"package": {"ecosystem": "crates.io", "name": "example", "version": "1"}, "vulnerabilities": advisories}]}]})
        self.assertEqual(len(findings), 3)
        self.assertEqual([item["id"] for item in security.blocked(findings, [])], ["unsound"])

    def test_expired_overlong_future_and_blanket_exceptions_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / ".github").mkdir()
            valid = {"tool": "osv", "id": "OSV-test", "target": "npm:example@1", "reason": "Reviewed reachability", "owner": "maintainer", "created": "2026-11-01", "expires": "2026-11-30"}
            for changes, fails in [({}, False), ({"expires": "2026-11-03"}, True), ({"expires": "2027-01-01"}, True), ({"created": "2026-11-10"}, True), ({"target": "*"}, True), ({"reason": " "}, True), ({"tool": "gitleaks"}, True)]:
                (root / ".github/security-exceptions.json").write_text(json.dumps({"exceptions": [{**valid, **changes}]}))
                if fails:
                    with self.assertRaises(ValueError):
                        security.exceptions(root, date(2026, 11, 3))
                else:
                    self.assertEqual(len(security.exceptions(root, date(2026, 11, 3))), 1)

    def test_sarif_suppression_cannot_bypass_repository_exception_policy(self):
        data = {"runs": [{"tool": {"driver": {"rules": [{"id": "unsafe", "properties": {"security-severity": "8.1"}}]}},
                          "results": [{"ruleId": "unsafe", "suppressions": [{"kind": "inSource"}], "locations": [{"physicalLocation": {"artifactLocation": {"uri": "app/test.rs"}}}]}]}]}
        findings = security.sarif_findings(data)
        self.assertEqual(len(security.blocked(findings, [])), 1)
        with self.assertRaises(ValueError):
            security.sarif_findings({"runs": []})

    def test_malformed_osv_and_service_failure_do_not_pass(self):
        with self.assertRaises(ValueError):
            security.osv_findings({})
        with patch.object(security, "request_json", side_effect=OSError("offline")), patch.object(security.subprocess, "run") as run:
            with tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary)
                (output / "osv.json").write_text('{"results": []}')
                run.return_value.returncode = 0
                with self.assertRaises(OSError):
                    security.scan(output)

    def test_scope_keeps_required_native_changes_and_docs_can_skip_heavy_jobs(self):
        self.assertFalse(scope.needs_native(["docs/guides/testing.md", "README.md"]))
        self.assertTrue(scope.needs_native(["docs/guides/testing.md", "app/src/main.ts"]))
        self.assertTrue(scope.needs_native([".github/workflows/security.yml"]))
        self.assertTrue(scope.needs_native(["tools/mon_release.py"]))


class DocumentationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        (self.root / "docs").mkdir()
        (self.root / "README.md").write_text("[Docs](docs/index.md)\n")
        (self.root / "docs/index.md").write_text('---\nokf_version: "0.2"\n---\n[Guide](guide.md)\n')
        self.guide = self.root / "docs/guide.md"
        self.guide.write_text('---\ntype: Guide\ntitle: Test\ndescription: Test\ntags: [test]\nstatus: stable\nsources:\n  - resource: https://example.com/spec\n---\n# 日本語の見出し\n\n[Self](#日本語の見出し)\n')

    def test_valid_bundle_and_broken_anchor_missing_link_or_orphan(self):
        self.assertEqual(docs.validate(self.root), [])
        self.guide.write_text(self.guide.read_text() + "\n[Bad](#does-not-exist)\n[Missing](missing.rs)\n")
        self.assertEqual(len(docs.validate(self.root)), 2)
        (self.root / "docs/orphan.md").write_text(self.guide.read_text())
        self.assertTrue(any("unreachable" in error for error in docs.validate(self.root)))

    def test_invalid_yaml_type_and_escaping_symlink_are_rejected(self):
        self.guide.write_text(self.guide.read_text().replace("type: Guide", "type: ''"))
        self.assertTrue(any("type" in error for error in docs.validate(self.root)))
        self.guide.write_text(self.guide.read_text() + "\n[Escape](../../outside.md)\n")
        self.assertTrue(any("escaping" in error for error in docs.validate(self.root)))

    def test_historical_sources_checked_at_commit_not_current_path(self):
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        old = self.root / "historical.rs"
        old.write_text("old source")
        subprocess.run(["git", "add", "."], cwd=self.root, check=True)
        subprocess.run(["git", "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "fixture"], cwd=self.root, check=True)
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=self.root).decode().strip()
        old.unlink()
        self.guide.write_text(self.guide.read_text().replace("https://example.com/spec", f"https://github.com/s-kodani/minuteson/blob/{commit}/historical.rs"))
        self.assertEqual(docs.validate(self.root), [])
        self.guide.write_text(self.guide.read_text().replace("historical.rs", "missing.rs"))
        self.assertTrue(any("source does not exist" in error for error in docs.validate(self.root)))


class ReleaseTests(unittest.TestCase):
    def test_draft_claim_is_idempotent_and_pins_source_before_dispatch(self):
        api = Provider()
        with patch.dict(os.environ, {"GITHUB_RUN_ID": "42"}):
            release.discover(api)
            release.discover(api)
        self.assertEqual(len(api.dispatches), 1)
        self.assertEqual(api.value["target_commitish"], SHA)
        self.assertEqual(api.dispatches[0]["ref"], "main")
        self.assertEqual(release.state(api.value)["fingerprint"], release.fingerprint(api.value))
        self.assertEqual(api.dispatches[0]["inputs"]["dry_run"], "true")

    def test_dispatch_failure_is_retained_without_automatic_duplicate_dispatch(self):
        api = Provider()
        original = api.call
        def call(endpoint, *args, **kwargs):
            if endpoint.endswith('/dispatches'):
                raise ValueError('temporary provider failure')
            return original(endpoint, *args, **kwargs)
        with patch.object(api, 'call', side_effect=call), patch.dict(os.environ, {"GITHUB_RUN_ID": "42"}):
            with self.assertRaises(ValueError):
                release.discover(api)
            self.assertEqual(release.state(api.value)["status"], "dispatch_failed")
            release.discover(api)

    def test_unchecked_draft_does_not_dispatch_and_non_main_source_fails(self):
        api = Provider()
        api.value["body"] = "Notes only"
        release.discover(api)
        self.assertEqual(api.dispatches, [])
        api.value["body"] = release.OPT_IN
        api.compare_status = "diverged"
        with self.assertRaises(ValueError):
            release.candidate(api, api.value)

    def test_version_and_draft_validation_reject_untrusted_values(self):
        release.validate_versions("v1.2.3", versions)
        for tag in ("v1.2.4", "v01.2.3", "v1.2.3-beta", "$(false)", "../v1.2.3"):
            with self.assertRaises(ValueError):
                release.validate_versions(tag, versions)
        api = Provider()
        for change in ({"target_commitish": "feature"}, {"draft": False}, {"prerelease": True}):
            with self.assertRaises(ValueError):
                release.validate_draft({**api.value, **change})

    def test_edit_after_claim_and_branch_dispatch_fail_preflight(self):
        api = Provider()
        with patch.dict(os.environ, {"GITHUB_RUN_ID": "42", "GITHUB_REF": "refs/heads/main"}):
            release.discover(api)
            release.preflight(api, 123, SHA)
            api.value["body"] += "\nEdited notes"
            with self.assertRaises(ValueError):
                release.preflight(api, 123, SHA)
        with patch.dict(os.environ, {"GITHUB_REF": "refs/heads/feature"}):
            with self.assertRaises(ValueError):
                release.preflight(api, 123, SHA)

    def test_publish_refuses_tampered_artifact_before_upload(self):
        api = Provider()
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"GITHUB_RUN_ID": "42", "GITHUB_REF": "refs/heads/main"}):
            release.discover(api)
            root = Path(temporary)
            expected = hashlib.sha256(b"expected").hexdigest()
            (root / "app.dmg").write_bytes(b"tampered")
            (root / "release-manifest.json").write_text(json.dumps({"sourceSha": SHA, "tag": "v1.2.3", "publicFiles": {"app.dmg": expected}}))
            (root / "SHA256SUMS").write_text("")
            (root / "provenance.json").write_text("{}")
            with self.assertRaises(ValueError):
                release.publish(api, 123, SHA, root)
            self.assertEqual(api.uploads, [])
            self.assertTrue(api.value["draft"])

    def test_failure_does_not_publish_and_same_draft_remains_retryable(self):
        api = Provider()
        with patch.dict(os.environ, {"GITHUB_RUN_ID": "42", "GITHUB_REF": "refs/heads/main"}):
            release.discover(api)
            release.failure(api, 123)
            self.assertTrue(api.value["draft"])
            self.assertEqual(release.state(api.value)["status"], "failed")
            release.preflight(api, 123, SHA)

    def test_publish_waits_for_complete_verified_assets(self):
        api = Provider()
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"GITHUB_RUN_ID": "42", "GITHUB_REF": "refs/heads/main"}):
            release.discover(api)
            root = Path(temporary)
            (root / "app.dmg").write_bytes(b"verified binary")
            expected = artifacts.digest(root / "app.dmg")
            manifest = {"sourceSha": SHA, "tag": "v1.2.3", "publicFiles": {"app.dmg": expected}}
            (root / "release-manifest.json").write_text(json.dumps(manifest))
            hashes = {"app.dmg": expected, "release-manifest.json": artifacts.digest(root / "release-manifest.json")}
            (root / "SHA256SUMS").write_text(''.join(f'{value}  {name}\n' for name, value in sorted(hashes.items())))
            (root / "provenance.json").write_text('{}')
            release.publish(api, 123, SHA, root)
            self.assertFalse(api.value['draft'])
            self.assertEqual(len(api.value['assets']), 4)
            self.assertEqual(release.state(api.value)['status'], 'published')

    def test_artifact_assembly_rejects_wrong_commit_and_changed_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "macos-receipt.json").write_text(json.dumps({"sourceSha": "2" * 40, "flavor": "macos"}))
            with patch.object(release, "validate_versions"):
                with self.assertRaises(ValueError):
                    artifacts.assemble(root, root / "public", SHA, "v1.2.3")


class HandoffTests(unittest.TestCase):
    def test_prepare_fetches_locked_dependencies_before_offline_build_hook(self):
        registry_ready = False

        def command(argv, **kwargs):
            nonlocal registry_ready
            if Path(argv[0]).name == "cargo":
                self.assertEqual(argv, ["cargo", "fetch", "--locked"])
                self.assertEqual(kwargs["cwd"], mac.REPOSITORY / "app/src-tauri")
                self.assertNotIn("APPLE_SIGNING_IDENTITY", kwargs["env"])
                registry_ready = True
            elif Path(argv[0]).name == "tauri":
                # This models the real beforeBuildCommand's offline metadata
                # lookup on a fresh runner, rather than assuming a warm cache.
                if not registry_ready:
                    raise ValueError("offline notice collection: no matching package named base64")
                built = Path(kwargs["env"]["CARGO_TARGET_DIR"]) / "release/bundle/macos/MinutesOn.app"
                built.mkdir(parents=True)
                (built / "source").write_bytes(b"synthetic application")
            elif Path(argv[0]).name == "ditto":
                with zipfile.ZipFile(argv[-1], "w") as output:
                    output.write(Path(argv[-2]) / "source", "MinutesOn.app/source")
            else:
                self.fail(f"unexpected preparation command: {argv[0]}")
            return b"", b""

        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary).resolve() / "prepared"
            with patch.object(mac.sys, "platform", "darwin"), patch.object(mac, "command", side_effect=command), \
                    patch.object(mac, "layout"), patch.object(mac, "verify_notices", return_value={"state": "collected_not_reviewed"}), \
                    patch.object(mac, "verify_asr_runtime"), patch.object(mac, "verify_inference_runtime"):
                result = mac.prepare(output, {"APPLE_SIGNING_IDENTITY": "must not reach dependency fetch"})
            self.assertEqual(result["archiveSha256"], artifacts.digest(output / "prepared-app.zip"))
            self.assertFalse((output / "build-cache").exists())

    def test_prepared_archive_rejects_traversal_and_escaping_links(self):
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=mac.REPOSITORY).decode().strip()
        config = mac.release_configuration()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for member, target in [("../escape", None), ("MinutesOn.app/Contents/link", "/tmp/escape"), ("MinutesOn.app/Contents/../escape", None)]:
                archive = root / "prepared-app.zip"
                with zipfile.ZipFile(archive, "w") as output:
                    if target:
                        info = zipfile.ZipInfo(member)
                        info.external_attr = (stat.S_IFLNK | 0o777) << 16
                        output.writestr(info, target)
                    else:
                        output.writestr(member, b"data")
                (root / "prepared-app.json").write_text(json.dumps({"commit": commit, "version": config["version"], "productName": config["productName"], "archiveSha256": artifacts.digest(archive), "notices": {}}))
                with self.assertRaises(ValueError):
                    mac.prepared_receipt(root)


class StoreTests(unittest.TestCase):
    def test_update_preserves_listings_and_pricing_and_replaces_packages_only(self):
        submission = {"id": "S1", "listings": {"ja-jp": {"title": "MinutesOn"}}, "pricing": {"priceId": "Free"},
                      "applicationPackages": [{"fileName": "old.msix", "fileStatus": "Published"}]}
        original = deepcopy(submission)
        changed = store.update_submission(submission, "new.msix")
        self.assertEqual(changed["listings"], original["listings"])
        self.assertEqual(changed["pricing"], original["pricing"])
        self.assertEqual(changed["applicationPackages"][0]["fileStatus"], "PendingDelete")
        self.assertEqual(changed["applicationPackages"][1]["fileStatus"], "PendingUpload")
        self.assertEqual(submission, original)

    def test_package_update_is_idempotent_and_preserves_target_metadata(self):
        for status in ("PendingUpload", "Uploaded"):
            with self.subTest(status=status):
                submission = {"id": "S1", "applicationPackages": [
                    {"fileName": "old.msix", "fileStatus": "PendingDelete", "id": "old-package"},
                    {"fileName": "new.msix", "fileStatus": status, "id": "new-package", "version": "1.2.3.0"},
                ]}
                original = deepcopy(submission)
                first = store.update_submission(submission, "new.msix")
                self.assertEqual(store.update_submission(first, "new.msix"), first)
                self.assertEqual(first["applicationPackages"][1], original["applicationPackages"][1])
                self.assertEqual(len(first["applicationPackages"]), 2)
                self.assertEqual(submission, original)

    def test_ambiguous_or_deleted_target_package_is_not_rewritten(self):
        package = {"fileName": "new.msix", "fileStatus": "PendingUpload"}
        for packages in ([package, deepcopy(package)], [{**package, "fileStatus": "PendingDelete"}]):
            with self.assertRaises(ValueError):
                store.update_submission({"applicationPackages": packages}, "new.msix")

    def test_existing_submission_is_polled_without_creating_another_or_downloading(self):
        api = Provider()
        api.value["draft"] = False
        record = {"status": "published", "storeSubmissionId": "S1", "store": "Certification", "sourceSha": SHA}
        api.save_state(api.value, record)
        provider = unittest.mock.Mock()
        provider.call.return_value = {"status": "Published"}
        with patch.object(store.subprocess, "run") as run:
            store.resume(api, provider, api.release(123), Path("unused"))
            run.assert_not_called()
        provider.call.assert_called_once_with("/submissions/S1/status")
        self.assertEqual(release.state(api.value)["store"], "Published")

    def test_azure_upload_cannot_send_packages_to_an_arbitrary_host(self):
        provider = object.__new__(store.Store)
        with self.assertRaises(ValueError):
            provider.upload("https://attacker.example/upload", Path("unused"))

    def test_interrupted_upload_resumes_same_submission_id(self):
        api = Provider()
        api.value['draft'] = False
        api.save_state(api.value, {'status': 'published', 'store': 'pending', 'sourceSha': SHA, 'buildRunId': '42'})
        package_bytes = b'synthetic inert MSIX'
        receipt = {'flavor': 'store', 'binary': 'test.msix', 'sha256': hashlib.sha256(package_bytes).hexdigest(), 'bytes': len(package_bytes)}
        manifest = {'sourceSha': SHA, 'tag': 'v1.2.3', 'artifacts': [receipt]}
        def download(argv, **kwargs):
            directory = Path(argv[argv.index('--dir') + 1])
            if 'release' in argv:
                (directory / 'release-manifest.json').write_text(json.dumps(manifest))
            else:
                (directory / 'test.msix').write_bytes(package_bytes)
        submission = {'id': 'S1', 'applicationPackages': [{'id': 'old-package', 'fileName': 'old.msix', 'fileStatus': 'Uploaded'}],
                      'fileUploadUrl': 'https://example.blob.core.windows.net/upload'}
        provider = unittest.mock.Mock()
        calls = []
        updates = []
        def call(suffix='', data=None, method=None):
            nonlocal submission
            calls.append((suffix, method))
            if suffix == '':
                return {'lastPublishedApplicationSubmission': {'id': 'old'}}
            if suffix == '/submissions/S1' and method == 'PUT':
                names = [item['fileName'] for item in data['applicationPackages']]
                self.assertEqual(len(names), len(set(names)))
                submission = deepcopy(data)
                # Persist the PUT as the real API does, including a package ID
                # that must survive the GET and subsequent retry.
                for item in submission['applicationPackages']:
                    if item['fileName'] == 'test.msix':
                        item.setdefault('id', 'new-package')
                updates.append(deepcopy(submission))
                return None
            if suffix == '/submissions' or suffix == '/submissions/S1':
                return deepcopy(submission)
            if suffix.endswith('/status'):
                return {'status': 'PendingCommit'}
            return None
        provider.call.side_effect = call
        provider.upload.side_effect = OSError('interrupted')
        with patch.object(store.subprocess, 'run', side_effect=download), patch.object(store, 'verify_manifest', return_value=(receipt, 42)):
            with tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(OSError):
                    store.resume(api, provider, api.release(123), Path(temporary))
            self.assertEqual(release.state(api.value)['storeSubmissionId'], 'S1')
            provider.upload.side_effect = None
            with tempfile.TemporaryDirectory() as temporary:
                store.resume(api, provider, api.release(123), Path(temporary))
        self.assertEqual(calls.count(('/submissions', 'POST')), 1)
        self.assertEqual(calls.count(('/submissions/S1/commit', 'POST')), 1)
        self.assertEqual(updates[0], updates[1])
        target = next(item for item in updates[1]['applicationPackages'] if item['fileName'] == 'test.msix')
        self.assertEqual(target['id'], 'new-package')
        self.assertEqual(target['fileStatus'], 'PendingUpload')


if __name__ == "__main__":
    unittest.main()
