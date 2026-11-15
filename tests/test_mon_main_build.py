"""Main output selection keeps the locally tested source and build immutable."""
from copy import deepcopy
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch
from urllib.parse import parse_qs, urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import mon_main_build as main
import mon_ci_scope as scope

SHA = '1' * 40


class CandidateTests(unittest.TestCase):
    def provider(self, *, count=1, expired=False, conclusion='success'):
        api = Mock()
        artifact = {'id': 7, 'name': main.PREFIX + SHA, 'expired': expired,
                    'workflow_run': {'id': 42, 'head_sha': SHA}}
        run = {'id': 42, 'path': main.WORKFLOW, 'head_sha': SHA, 'head_branch': 'main',
               'event': 'push', 'status': 'completed', 'conclusion': conclusion}
        api.call.side_effect = lambda endpoint: ({'artifacts': [deepcopy(artifact) for _ in range(count)]}
                                                if '/artifacts?' in endpoint else deepcopy(run))
        return api

    def test_exact_source_expiration_and_ambiguity(self):
        self.assertEqual(main.find_candidate(self.provider(), SHA), {'artifactId': 7, 'runId': 42, 'sourceSha': SHA})
        self.assertIsNone(main.find_candidate(self.provider(count=0), SHA))
        self.assertIsNone(main.find_candidate(self.provider(conclusion='failure'), SHA))
        for provider in (self.provider(expired=True), self.provider(count=2)):
            with self.assertRaises(ValueError): main.find_candidate(provider, SHA)
        with self.assertRaises(ValueError):
            main.find_candidate(self.provider(conclusion='failure'), SHA, completed=False)

    def test_main_guard_keeps_existing_candidate_instead_of_rebuilding(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
            'GITHUB_REF': 'refs/heads/main', 'GITHUB_SHA': SHA, 'GITHUB_EVENT_NAME': 'workflow_dispatch',
            'GITHUB_OUTPUT': str(Path(temporary) / 'output'),
        }):
            main.guard(self.provider())
            values = dict(line.split('=', 1) for line in Path(os.environ['GITHUB_OUTPUT']).read_text().splitlines())
            self.assertEqual(values['build'], 'false')
            self.assertEqual(values['reuse'], 'true')
            self.assertEqual(values['source_sha'], SHA)

    def test_docs_main_does_not_lookup_or_build_a_candidate(self):
        api = Mock()
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
            'GITHUB_REF': 'refs/heads/main', 'GITHUB_SHA': SHA, 'GITHUB_EVENT_NAME': 'push',
            'GITHUB_OUTPUT': str(Path(temporary) / 'output'),
        }), patch.object(main, 'changed_paths', return_value=[
            'README.md', 'AGENTS.md', '.github/pull_request_template.md',
            '.agents/skills/desktop-ui-quality/agents/openai.yaml',
            'third-party/notices/README.md', 'tests/fixtures/audio-import/README.md',
            'evaluation/runs/.gitkeep',
        ]):
            main.guard(api)
            api.call.assert_not_called()
            self.assertIn('build=false', Path(os.environ['GITHUB_OUTPUT']).read_text())

    def test_release_form_targets_the_tested_commit_even_if_main_advances(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
            'GITHUB_REPOSITORY': 'owner/repo', 'GITHUB_RUN_ID': '42', 'MON_SOURCE_SHA': SHA,
            'MON_TAG': 'v1.2.3', 'MON_ARTIFACT_URL': 'https://github.com/owner/repo/actions/runs/42/artifacts/7',
            'GITHUB_STEP_SUMMARY': str(Path(temporary) / 'summary'),
        }):
            main.summary()
            text = Path(os.environ['GITHUB_STEP_SUMMARY']).read_text()
            url = next(part.split(')')[0] for part in text.split('](') if '/releases/new?' in part)
            values = parse_qs(urlsplit(url).query)
            self.assertEqual(values['target'], [SHA])
            self.assertEqual(values['tag'], ['v1.2.3'])


class ScopeTests(unittest.TestCase):
    def test_docs_skip_all_expensive_checks_and_runtime_markdown_does_not(self):
        self.assertFalse(scope.select(['README.md', 'docs/spec/storage.md'])['has_codeql'])
        for path in ('app/src/prompt.md', 'third-party/LICENSE.md', '.github/workflows/new.yml', 'unknown.txt'):
            self.assertTrue(scope.select([path])['code'])

    def test_development_docs_and_agent_metadata_skip_expensive_pr_and_main_checks(self):
        paths = (
            'AGENTS.md', '.github/pull_request_template.md',
            '.agents/skills/desktop-ui-quality/SKILL.md',
            '.agents/skills/desktop-app-updates/references/macos.md',
            '.agents/skills/desktop-ui-quality/agents/openai.yaml',
            'tests/fixtures/audio-import/README.md',
            'third-party/model-licenses/README.md',
            'third-party/notices/README.md',
            'third-party/runtime-notices/README.md',
            'evaluation/runs/.gitkeep',
        )
        for path in paths:
            for on_main in (False, True):
                with self.subTest(path=path, main=on_main):
                    result = scope.select([path], main=on_main)
                    for key in (*scope.CHECKS, 'code', 'has_codeql'):
                        self.assertFalse(result[key], key)
                    self.assertEqual(result['codeql']['include'], [])

    def test_documentation_exceptions_keep_executable_assets_and_unknown_paths_checked(self):
        paths = (
            '.agents/skills/example/scripts/check.py',
            '.agents/skills/example/agents/settings.yaml',
            '.agents/skills/example/scripts/agents/openai.yaml',
            '.github/workflows/security.yml', '.github/security-exceptions.json',
            'app/model-catalog/model-license-policy.json', 'app/package-lock.json',
            'tests/fixtures/audio-import/structure-chapter.m4a',
            'third-party/notices/objc2-7b1abfd-LICENSE.md',
            'third-party/notices/index.json', 'third-party/runtime-notices/index.json',
            'third-party/model-licenses/model.txt',
            'third-party/new-materials/README.md', 'unknown/AGENTS.md',
            'evaluation/config.toml',
        )
        for path in paths:
            with self.subTest(path=path):
                result = scope.select([path])
                self.assertTrue(all(result[key] for key in scope.CHECKS))
                self.assertEqual(len(result['codeql']['include']), 6)
        for path in ('app/src/prompt.md', 'app/src-tauri/prompt.md',
                     'tests/fixtures/audio-import/manifest.json'):
            with self.subTest(path=path):
                self.assertTrue(scope.select([path])['code'])

    def test_docs_do_not_hide_code_changes_or_explicit_full_validation(self):
        for path in ('app/src/main.tsx', 'app/src-tauri/src/storage.rs',
                     'tools/mon_ci_scope.py', '.github/workflows/security.yml'):
            with self.subTest(path=path):
                self.assertEqual(scope.select(['AGENTS.md', path]), scope.select([path]))
        result = scope.select(['AGENTS.md'], full=True)
        self.assertTrue(all(result[key] for key in (*scope.CHECKS, 'code', 'has_codeql')))
        self.assertEqual(len(result['codeql']['include']), 6)

    def test_python_frontend_rust_and_contracts_select_corresponding_languages(self):
        cases = {'tools/mon_release.py': {'python'}, 'app/src/main.tsx': {'javascript-typescript'},
                 'app/src-tauri/src/storage.rs': {'rust'},
                 'crates/mon-contracts/src/model.rs': {'rust', 'javascript-typescript'}}
        for path, expected in cases.items():
            result = scope.select([path])
            self.assertEqual({entry['language'] for entry in result['codeql']['include']}, expected)
        self.assertEqual(len(scope.select(['app/src/main.tsx'], main=True)['codeql']['include']), 6)
        self.assertTrue(scope.select(['app/src-tauri/src/storage.rs', 'docs/moved.md'])['native'])
