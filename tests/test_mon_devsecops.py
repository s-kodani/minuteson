"""Behavioral tests for security policy, documentation, and release boundaries.

All provider mutations are in-memory fakes. No signing, publication or Store
submission is performed by this suite.
"""
import base64
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
import mon_main_build as main_build
import mon_store_provenance as provenance
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
                      "prerelease": False, "name": "Release", "body": "Release notes", "author": {"login": "owner"}, "assets": []}
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

    def test_main_tests_and_builds_once_before_saving_candidate(self):
        workflow = yaml.safe_load((release.ROOT / '.github/workflows/release-build.yml').read_text())
        events = workflow.get('on', workflow.get(True))
        self.assertEqual(events['push']['branches'], ['main'])
        self.assertNotIn('release', events)
        self.assertNotIn('release_id', events.get('workflow_dispatch') or {})
        self.assertTrue(workflow['jobs']['ci']['with']['full'])
        self.assertIn('build_release', workflow['jobs']['ci']['with'])
        self.assertIn('needs.guard.outputs.build', workflow['jobs']['security']['with']['full'])
        self.assertEqual(set(workflow['jobs']['candidate']['needs']), {'guard', 'ci', 'security', 'macos-sign'})
        self.assertNotIn('publish', workflow['jobs'])
        for name in ('macos-sign', 'candidate'):
            self.assertEqual(workflow['jobs'][name]['steps'][0]['with']['ref'], '${{ github.sha }}')
        steps = workflow['jobs']['candidate']['steps']
        self.assertEqual(steps[-1]['name'], provenance.CANDIDATE_STEP)
        upload = next(step for step in steps if step.get('id') == 'candidate')
        self.assertEqual(upload['with']['retention-days'], 30)
        self.assertNotIn('overwrite', upload['with'])
        commands = '\n'.join(step.get('run', '') for step in steps)
        self.assertNotIn('mon_release.py publish', commands)

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

    def test_only_draft_controller_can_attach_and_never_dispatches_a_build(self):
        workflow = yaml.safe_load((release.ROOT / '.github/workflows/release-controller.yml').read_text())
        self.assertEqual(workflow['permissions']['contents'], 'write')
        self.assertEqual(workflow['permissions']['actions'], 'read')
        commands = '\n'.join(step.get('run', '') for step in workflow['jobs']['discover']['steps'])
        self.assertIn('mon_release.py discover', commands)
        self.assertNotIn('publish', commands)


class DesktopWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = yaml.safe_load((release.ROOT / ".github/workflows/desktop-platform-build.yml").read_text())
        self.events = self.workflow.get("on", self.workflow.get(True))

    @staticmethod
    def expression(value, context):
        """Evaluate the small boolean subset used by the native step guards."""
        value = value.removeprefix("${{ ").removesuffix(" }}")
        value = re.sub(r"\b(?:inputs|github|runner|env|steps|matrix)(?:\.[A-Za-z_][A-Za-z_0-9-]*)+",
                       lambda match: repr(context[match[0]]), value)
        value = value.replace("&&", " and ").replace("||", " or ")
        return eval(value, {"__builtins__": {}}, {"always": lambda: True, "success": lambda: True})

    def native_steps(self, event="pull_request", full=False, suite="", operating_system="Windows", rust_cache_hit=False,
                     build_release=False, workflow="Desktop platform validation"):
        context = {"inputs.full": full, "inputs.suite": suite, "github.event_name": event,
                   "inputs.build_release": build_release,
                   "runner.os": operating_system, "github.ref": "refs/heads/main",
                   "github.workflow": workflow, "inputs.source_ref": "", "github.sha": SHA,
                   "steps.native_downloads.outputs.cache-hit": "false",
                   **{f"steps.{name}_cache.outputs.cache-hit": "false" for name in (
                       "windows_runtime", "windows_capture", "macos_capture", "macos_workers")},
                   "steps.rust_cache.outputs.cache-hit": str(rust_cache_hit).lower()}
        native = self.workflow["jobs"]["native"]
        context["env.MON_CI_FULL"] = str(self.expression(native["env"]["MON_CI_FULL"], context)).lower()
        context["env.MON_SAVE_CI_CACHE"] = str(self.expression(native["env"]["MON_SAVE_CI_CACHE"], context)).lower()
        context["env.MON_BUILD_RELEASE"] = str(build_release).lower()
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
        qa_dpi = names.index('Verify DPI declarations in the linked Windows installer executable')
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

    def test_main_collects_final_exe_before_store_build_in_the_validated_job(self):
        steps = self.native_steps(event='push', full=True, build_release=True, workflow='Main build')
        commands = '\n'.join(step.get('run', '') for step in steps)
        self.assertNotIn('tauri:windows:qa:build', commands)
        self.assertEqual(commands.count('tauri:windows:unsigned:build'), 1)
        self.assertEqual(commands.count('tauri:windows:store:build'), 1)
        self.assertLess(commands.index('collect --flavor windows'), commands.index('tauri:windows:store:build'))
        self.assertIn('--suite full', commands)
        self.assertNotIn('--all-targets', commands)
        uploads = [step['with'] for step in steps if step.get('uses', '').startswith('actions/upload-artifact@')]
        for name in ('release-windows', 'release-store'):
            upload = next(item for item in uploads if item['name'] == name)
            self.assertEqual(upload['retention-days'], 30)
        mac_steps = self.native_steps(event='push', full=True, build_release=True, workflow='Main build', operating_system='macOS')
        self.assertTrue(any('mon_macos_release.py --prepare' in step.get('run', '') for step in mac_steps))

    def test_installer_dpi_uses_the_selected_build_output_and_store_keeps_its_own(self):
        for build_release, target in ((False, 'target'), (True, 'target-unsigned')):
            with self.subTest(build_release=build_release):
                steps = self.native_steps(full=True, build_release=build_release)
                installer = next(s for s in steps if s.get('name') == 'Verify DPI declarations in the linked Windows installer executable')
                selected = self.expression(installer['env']['MON_WINDOWS_DPI_TARGET'], {
                    'env.MON_BUILD_RELEASE': str(build_release).lower(),
                })
                self.assertEqual(selected, target)
                self.assertIn('app/src-tauri/$env:MON_WINDOWS_DPI_TARGET/', installer['run'])
                self.assertIn('MON_WINDOWS_DPI_BINARY', installer['run'])
                self.assertIn('test_mon_windows_dpi.py', installer['run'])
                store = next(s for s in steps if s.get('name') == 'Verify DPI declarations in the linked Windows Store executable')
                self.assertIn('app/src-tauri/target/x86_64-pc-windows-msvc/release/mon-desktop.exe', store['run'])
                self.assertNotIn('MON_WINDOWS_DPI_TARGET', store.get('env', {}))

    def test_final_packaging_requires_main_and_full_validation(self):
        step = next(s for s in self.workflow['jobs']['changes']['steps'] if s.get('name') == 'Validate main packaging request')
        script = '\n'.join(step['run'].splitlines()[1:-1])
        for build, full, ref in (('true', 'false', 'refs/heads/main'), ('true', 'true', 'refs/heads/feature'),
                                 ('true', 'true', 'refs/heads/main'), ('false', 'false', 'refs/pull/1/merge')):
            with self.subTest(build=build, full=full, ref=ref), patch.dict(os.environ, {
                    'MON_BUILD_RELEASE': build, 'MON_CI_FULL': full, 'GITHUB_REF': ref}):
                if build == 'true' and (full != 'true' or ref != 'refs/heads/main'):
                    with self.assertRaises(SystemExit): exec(compile(script, 'main-packaging', 'exec'), {})
                else:
                    exec(compile(script, 'main-packaging', 'exec'), {})

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
                    self.assertIn("--suite quick --processes 2 --threads 4", commands)
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
            self.assertIn("--suite full", windows)
            macos = self.native_commands(event, full, suite, "macOS")
            self.assertIn("build-core.sh", macos)
            self.assertIn("swift test", macos)
            self.assertIn("--suite full --swift-fixture", macos)
            self.assertNotIn("--suite quick", windows + macos)
            self.assertNotIn("--processes", windows + macos)
            self.assertNotIn("--threads", windows + macos)
        self.assertIn("--suite quick", self.native_commands("workflow_dispatch", False, "quick"))

    def test_final_binaries_are_saved_only_by_explicit_main_packaging(self):
        for definition in self.workflow['jobs'].values():
            for step in definition.get('steps', []):
                if step.get('uses', '').startswith('actions/upload-artifact@'):
                    if step['with']['name'] in ('prepared-macos', 'release-windows', 'release-store'):
                        self.assertIn("env.MON_BUILD_RELEASE == 'true'", step['if'])
                        self.assertIn("env.MON_CI_FULL == 'true'", step['if'])

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

    def test_full_native_schedule_is_removed_and_main_owns_push_validation(self):
        daily = yaml.safe_load((release.ROOT / '.github/workflows/desktop-daily.yml').read_text())
        self.assertEqual(set(daily.get('on', daily.get(True))), {'workflow_dispatch'})
        self.assertTrue(daily['jobs']['validate']['with']['full'])
        self.assertNotIn('push', self.events)
        security = yaml.safe_load((release.ROOT / '.github/workflows/security.yml').read_text())
        events = security.get('on', security.get(True))
        self.assertNotIn('push', events)
        self.assertIn('schedule', events)
        main = yaml.safe_load((release.ROOT / '.github/workflows/release-build.yml').read_text())
        self.assertTrue(main['jobs']['ci']['with']['full'])

    def test_gate_accepts_only_planned_skips_and_fails_when_scope_is_unavailable(self):
        gate = self.workflow['jobs']['gate']
        self.assertEqual(gate['if'], 'always()')
        script = '\n'.join(gate['steps'][0]['run'].splitlines()[1:-1])
        for affected in ((), ('PYTHON',), ('BROWSER',), ('NATIVE',), ('PYTHON', 'BROWSER', 'NATIVE')):
            values = {'MON_CHANGES': 'success'}
            for job in ('PYTHON', 'BROWSER', 'NATIVE'):
                values['MON_REQUIRE_' + job] = str(job in affected).lower()
                values['MON_' + job] = 'success' if job in affected else 'skipped'
            with patch.dict(os.environ, values): exec(compile(script, 'ci-gate', 'exec'), {})
            for key in ('MON_CHANGES', *('MON_' + job for job in affected)):
                for result in ('failure', 'cancelled', 'skipped'):
                    with self.subTest(affected=affected, key=key, result=result), patch.dict(os.environ, {**values, key: result}):
                        with self.assertRaises(SystemExit): exec(compile(script, 'ci-gate', 'exec'), {})
            with patch.dict(os.environ, {**values, 'MON_REQUIRE_NATIVE': ''}):
                with self.assertRaises(SystemExit): exec(compile(script, 'ci-gate', 'exec'), {})

    def test_scoped_jobs_and_gate_use_the_same_classification_output(self):
        for job in ('python', 'browser', 'native'):
            self.assertEqual(self.workflow['jobs'][job]['if'], f"needs.changes.outputs.{job} == 'true'")
            self.assertEqual(self.workflow['jobs']['gate']['env']['MON_REQUIRE_' + job.upper()],
                             '${{ needs.changes.outputs.' + job + ' }}')

    def test_security_gate_rejects_scope_failure_and_unplanned_codeql_skips(self):
        workflow = yaml.safe_load((release.ROOT / '.github/workflows/security.yml').read_text())
        gate = workflow['jobs']['gate']
        self.assertEqual(gate['if'], 'always()')
        script = '\n'.join(gate['steps'][0]['run'].splitlines()[1:-1])
        values = {'MON_CHANGES': 'success', 'MON_POLICY': 'success', 'MON_DOCS': 'success',
                  'MON_CODEQL': 'success', 'MON_HAS_CODEQL': 'true', 'MON_PRIVATE': 'false'}
        for change in ({}, {'MON_HAS_CODEQL': 'false', 'MON_CODEQL': 'skipped'}):
            with patch.dict(os.environ, {**values, **change}): exec(compile(script, 'security-gate', 'exec'), {})
        for change in ({'MON_CHANGES': 'failure'}, {'MON_POLICY': 'cancelled'}, {'MON_DOCS': 'skipped'},
                       {'MON_HAS_CODEQL': ''}, {'MON_CODEQL': 'skipped'}):
            with patch.dict(os.environ, {**values, **change}):
                with self.assertRaises(SystemExit): exec(compile(script, 'security-gate', 'exec'), {})

    def test_docs_and_full_scope_run_through_the_actual_scope_program(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'output'
            with patch.dict(os.environ, {'MON_BASE': SHA, 'MON_FULL': 'false', 'GITHUB_OUTPUT': str(output)}), \
                    patch.object(scope.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, b'docs/guides/testing.md\0README.md\0')) as diff:
                scope.main()
                self.assertIn('--no-renames', diff.call_args.args[0])
            values = dict(line.split('=', 1) for line in output.read_text().splitlines())
            self.assertTrue(all(values[key] == 'false' for key in (*scope.CHECKS, 'has_codeql', 'code')))
            output.unlink()
            with patch.dict(os.environ, {'MON_BASE': SHA, 'MON_FULL': 'true', 'GITHUB_OUTPUT': str(output)}), patch.object(scope.subprocess, 'run') as diff:
                scope.main()
                diff.assert_not_called()
            values = dict(line.split('=', 1) for line in output.read_text().splitlines())
            self.assertTrue(all(values[key] == 'true' for key in scope.CHECKS))


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

    def test_scope_selects_components_and_unknown_changes_fail_closed(self):
        cases = [(['docs/guides/testing.md', 'README.md'], set()),
                 (['app/src/main.ts'], {'browser'}), (['tools/mon_release.py'], {'python'}),
                 (['app/src-tauri/src/storage.rs'], {'native'}),
                 (['crates/mon-contracts/src/model.rs'], {'browser', 'native'}),
                 (['.github/workflows/security.yml'], set(scope.CHECKS)),
                 (['new-directory/file.md'], set(scope.CHECKS)),
                 (['third-party/notice.md'], set(scope.CHECKS))]
        for paths, expected in cases:
            self.assertEqual({key for key in scope.CHECKS if scope.select(paths)[key]}, expected, paths)
        self.assertTrue(all(scope.select(['app/src/main.ts'], main=True)[key] for key in scope.CHECKS))


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
    def test_github_client_compares_a_resolved_commit_to_main(self):
        api = release.GitHub(repository="owner/repo", token="test-token")
        with patch.object(release.urllib.request, "urlopen") as urlopen:
            urlopen.return_value.__enter__.return_value.read.return_value = b'{"status":"identical"}'
            self.assertEqual(api.call(f"/compare/{SHA}...main"), {"status": "identical"})
            request = urlopen.call_args.args[0]
            self.assertEqual(request.full_url, f"https://api.github.com/repos/owner/repo/compare/{SHA}...main")
            with self.assertRaisesRegex(ValueError, "invalid GitHub endpoint"):
                api.call("/compare/not-a-commit...main")
            urlopen.assert_called_once()

    def test_draft_state_patch_keeps_the_release_tag(self):
        api = release.GitHub(repository="owner/repo", token="test-token")
        draft = Provider().value
        with patch.object(api, "call") as call:
            api.save_state(draft, {"status": "claimed"}, target_commitish=SHA)
            endpoint, payload, method = call.call_args.args
            self.assertEqual((endpoint, method), ("/releases/123", "PATCH"))
            self.assertEqual(payload["tag_name"], "v1.2.3")
            self.assertEqual(payload["target_commitish"], SHA)
            api.save_state(draft, {"status": "claimed"})
            self.assertEqual(call.call_args.args[1]["tag_name"], "v1.2.3")

    @staticmethod
    def candidate_files(root):
        (root / 'app.dmg').write_bytes(b'verified binary')
        expected = artifacts.digest(root / 'app.dmg')
        manifest = {'schemaVersion': 2, 'sourceSha': SHA, 'tag': 'v1.2.3',
                    'producer': {'kind': 'main-build', 'runId': 42, 'runAttempt': 1, 'event': 'push'},
                    'publicFiles': {'app.dmg': expected}}
        (root / 'release-manifest.json').write_text(json.dumps(manifest))
        hashes = {'app.dmg': expected, 'release-manifest.json': artifacts.digest(root / 'release-manifest.json')}
        (root / 'SHA256SUMS').write_text(''.join(f'{value}  {name}\n' for name, value in sorted(hashes.items())))
        (root / 'provenance.json').write_text('{}')

    def test_draft_without_checkbox_attaches_idempotently_and_never_publishes(self):
        api = Provider()
        saved = {'artifactId': 7, 'runId': 42, 'sourceSha': SHA}
        def download(argv, **kwargs):
            self.candidate_files(Path(argv[argv.index('--dir') + 1]))
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main'}), \
                patch.object(main_build, 'find_candidate', return_value=saved), \
                patch.object(release.subprocess, 'run', side_effect=download) as run, \
                patch.object(provenance, 'verify_manifest', return_value=({}, 42)):
            release.discover(api)
            api.value['body'] += '\nEdited release notes'
            release.discover(api)
        self.assertEqual(api.value['target_commitish'], SHA)
        self.assertTrue(api.value['draft'])
        self.assertEqual(release.state(api.value)['status'], 'attached')
        self.assertEqual(len(api.uploads), 4)
        self.assertEqual(api.dispatches, [])
        run.assert_called_once()

    def test_partial_attachment_resumes_but_conflicting_or_unexpected_assets_stop(self):
        saved = {'artifactId': 7, 'runId': 42, 'sourceSha': SHA}
        with tempfile.TemporaryDirectory() as temporary, patch.object(provenance, 'verify_manifest', return_value=({}, 42)):
            root = Path(temporary)
            self.candidate_files(root)
            api = Provider()
            api.value['target_commitish'] = SHA
            expected = release.asset_metadata({'app.dmg': root / 'app.dmg'})
            api.value['assets'] = [{'name': 'app.dmg', **expected['app.dmg']}]
            release.attach(api, 123, SHA, saved, root)
            self.assertEqual(len(api.uploads), 3)
            self.assertTrue(api.value['draft'])
            for changes in ({'name': 'unknown.exe'}, {'digest': 'sha256:bad'}, {'size': 0}):
                api = Provider()
                api.value['target_commitish'] = SHA
                api.value['assets'] = [{'name': 'app.dmg', **expected['app.dmg'], **changes}]
                with self.assertRaises(ValueError): release.attach(api, 123, SHA, saved, root)
                self.assertEqual(api.uploads, [])

    def test_missing_candidate_or_changed_source_never_dispatches_a_rebuild(self):
        api = Provider()
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main'}), patch.object(main_build, 'find_candidate', return_value=None):
            with self.assertRaises(ValueError): release.discover(api)
        self.assertEqual(api.dispatches, [])
        self.assertEqual(api.uploads, [])
        api.compare_status = 'diverged'
        with self.assertRaises(ValueError): release.candidate(api, api.value)
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/feature'}):
            with self.assertRaises(ValueError): release.discover(api)

    def test_changed_bytes_and_published_release_are_rejected_before_upload(self):
        api = Provider()
        api.value['target_commitish'] = SHA
        saved = {'artifactId': 7, 'runId': 42, 'sourceSha': SHA}
        with tempfile.TemporaryDirectory() as temporary, patch.object(provenance, 'verify_manifest', return_value=({}, 42)):
            root = Path(temporary)
            self.candidate_files(root)
            (root / 'app.dmg').write_bytes(b'tampered')
            with self.assertRaises(ValueError): release.attach(api, 123, SHA, saved, root)
            self.candidate_files(root)
            api.value['draft'] = False
            with self.assertRaises(ValueError): release.attach(api, 123, SHA, saved, root)
        self.assertEqual(api.uploads, [])

    def test_version_and_draft_validation_reject_untrusted_values(self):
        release.validate_versions('v1.2.3', versions)
        for tag in ('v1.2.4', 'v01.2.3', 'v1.2.3-beta', '$(false)', '../v1.2.3'):
            with self.assertRaises(ValueError): release.validate_versions(tag, versions)
        api = Provider()
        for change in ({'target_commitish': 'feature'}, {'draft': False}, {'prerelease': True}):
            with self.assertRaises(ValueError): release.validate_draft({**api.value, **change})

    def test_artifact_assembly_rejects_wrong_commit_and_changed_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "macos-receipt.json").write_text(json.dumps({"sourceSha": "2" * 40, "flavor": "macos"}))
            with patch.object(release, "validate_versions"):
                with self.assertRaises(ValueError):
                    artifacts.assemble(root, root / "public", SHA, "v1.2.3")

    def test_main_assembly_binds_all_flavors_to_one_run_and_keeps_msix_private(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
            'MON_MAIN_CANDIDATE': 'true', 'GITHUB_RUN_ID': '42', 'GITHUB_RUN_ATTEMPT': '2', 'GITHUB_EVENT_NAME': 'push',
        }):
            root = Path(temporary)
            for flavor, suffix, status in (('macos', 'dmg', 'developer_id_signed_and_notarized'),
                                           ('windows', 'exe', 'unsigned_verified'),
                                           ('store', 'msix', 'store_payload_verified_unsigned')):
                binary, bom = root / ('app.' + suffix), root / (flavor + '.cdx.json')
                binary.write_bytes(b'inert fixture')
                bom.write_text('{}')
                (root / (flavor + '-receipt.json')).write_text(json.dumps({
                    'schemaVersion': 1, 'sourceSha': SHA, 'flavor': flavor, 'verification': status,
                    'binary': binary.name, 'sha256': artifacts.digest(binary), 'bytes': binary.stat().st_size,
                    'sbom': bom.name, 'sbomSha256': artifacts.digest(bom),
                }))
            tag = 'v' + json.loads((artifacts.ROOT / 'app/package.json').read_text())['version']
            artifacts.assemble(root, root / 'public', SHA, tag)
            manifest = json.loads((root / 'public/release-manifest.json').read_text())
            self.assertEqual(manifest['schemaVersion'], 2)
            self.assertEqual(manifest['producer'], {'kind': 'main-build', 'runId': 42, 'runAttempt': 2, 'event': 'push'})
            self.assertFalse((root / 'public/app.msix').exists())
            self.assertIn('store.cdx.json', manifest['publicFiles'])
            self.assertEqual(manifest['artifacts'][2]['sha256'], artifacts.digest(root / 'app.msix'))


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
        provider.read_only = False
        with self.assertRaises(ValueError):
            provider.upload("https://attacker.example/upload", Path("unused"))

    def test_interrupted_upload_resumes_same_submission_id(self):
        api = Provider()
        api.value['draft'] = False
        api.save_state(api.value, {'status': 'published', 'store': 'pending', 'sourceSha': SHA, 'buildRunId': '42'})
        package_bytes = b'synthetic inert MSIX'
        receipt = {'sourceSha': SHA, 'flavor': 'store', 'binary': 'test.msix', 'sha256': hashlib.sha256(package_bytes).hexdigest(), 'bytes': len(package_bytes)}
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
