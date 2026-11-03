"""Command boundary snapshots fail closed when native signatures change."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('mon_contracts_tool', ROOT / 'tools/mon_contracts.py')
contracts = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contracts)

SOURCE = '''
#[tauri::command]
pub(crate) async fn inspect(
    window: tauri::WebviewWindow,
    state: State<'_, std::sync::Arc<Storage>>,
    operation_id: String,
    expected: Option<Vec<u64>>,
) -> Result<Vec<storage::Report>, mon_contracts::AppError> { todo!() }
'''


class CommandContractTests(unittest.TestCase):
    def test_repository_check_does_not_depend_on_windows_default_encoding(self):
        result = subprocess.run([sys.executable, '-X', 'utf8=0', str(ROOT / 'tools/mon_contracts.py'), '--check'],
                                cwd=ROOT, capture_output=True, text=True, encoding='utf8', errors='replace', timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def fixture(self, source=SOURCE):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        native = root / 'app/src-tauri'
        (native / 'src').mkdir(parents=True)
        file = native / 'src/lib.rs'
        file.write_text(source)
        (native / 'permissions').mkdir()
        (root / 'app/src/generated').mkdir(parents=True)
        (native / 'command_registry.json').write_text(json.dumps(dict(schemaVersion=1, commands=[dict(
            name='inspect', handler='inspect', roles=['main'])])))
        contracts.registry_commands(root, synchronize=True)
        return root, file

    def test_public_args_exclude_injection_and_preserve_optional_nested_types_and_error(self):
        signature = contracts.command_signatures(SOURCE)['inspect']
        self.assertEqual(signature, {
            'async': True,
            'arguments': [
                {'name': 'operation_id', 'wireName': 'operationId', 'type': 'String', 'optional': False},
                {'name': 'expected', 'wireName': 'expected', 'type': 'Option<Vec<u64>>', 'optional': True}],
            'response': {'type': 'Result<Vec<storage::Report>,mon_contracts::AppError>',
                         'ok': 'Vec<storage::Report>', 'error': 'mon_contracts::AppError'}})
        schema = contracts.typescript_commands([{'name': 'inspect', 'signature': signature}])
        self.assertIn('"operationId": string; "expected"?: (Array<number>) | null', schema)
        self.assertNotIn('"window":', schema)
        self.assertNotIn('"state":', schema)

    def test_signature_name_order_type_success_error_and_async_drift_each_fail(self):
        changes = [
            ('operation_id: String', 'operation_key: String'),
            ('operation_id: String,\n    expected: Option<Vec<u64>>,', 'expected: Option<Vec<u64>>,\n    operation_id: String,'),
            ('operation_id: String', 'operation_id: u64'),
            ('Vec<storage::Report>', 'Option<storage::Report>'),
            ('mon_contracts::AppError', 'String'),
            ('async fn', 'fn'),
            ('state: State<\'_, std::sync::Arc<Storage>>', 'state: std::sync::Arc<Storage>'),
        ]
        for before, after in changes:
            with self.subTest(after=after):
                root, source = self.fixture()
                source.write_text(SOURCE.replace(before, after))
                with self.assertRaisesRegex(ValueError, 'signature differs'):
                    contracts.products(root)

    def test_comments_layout_and_fake_string_attributes_cannot_change_snapshots(self):
        source = SOURCE.replace('operation_id: String', 'operation_id /* nested /* note */ comment */ : String')
        source = source.replace('todo!()', '''let fake = r###"#[tauri::command] fn fake(arg: String) -> Result<(), String> {}"###; todo!()''')
        self.assertEqual(contracts.command_signatures(source), contracts.command_signatures(SOURCE))
        self.assertEqual(contracts.command_signatures('// #[tauri::command] fn fake() {}\n' + source), contracts.command_signatures(SOURCE))

    def test_missing_attribute_and_new_unregistered_handler_fail(self):
        root, source = self.fixture()
        source.write_text(SOURCE.replace('#[tauri::command]', ''))
        with self.assertRaisesRegex(ValueError, 'lacks'):
            contracts.products(root)
        source.write_text(SOURCE + '\n#[tauri::command]\nfn ungranted() {}')
        with self.assertRaisesRegex(ValueError, 'absent from registry'):
            contracts.products(root)

    def test_explicit_sync_changes_only_signature_and_retains_role_grants(self):
        root, source = self.fixture()
        path = root / 'app/src-tauri/command_registry.json'
        before = json.loads(path.read_text())
        source.write_text(SOURCE.replace('Option<Vec<u64>>', 'Option<String>'))
        contracts.registry_commands(root, synchronize=True)
        after = json.loads(path.read_text())
        before['commands'][0]['signature'] = copy.deepcopy(after['commands'][0]['signature'])
        self.assertEqual(before, after)
        products = contracts.products(root)
        self.assertIn('"expected"?: (string) | null', products[root / 'app/src/generated/commands.ts'])
        self.assertIn('"inspect" => matches!(role, "main")', products[root / 'app/src-tauri/src/command_roles.rs'])

    def test_unsupported_syntax_and_wire_aliases_fail_instead_of_guessing(self):
        for source in [
            SOURCE.replace('fn inspect(', 'fn inspect<T>('),
            SOURCE.replace('#[tauri::command]', '#[tauri::command(rename_all="unknown")]'),
            SOURCE.replace('expected: Option<Vec<u64>>', 'operationId: Option<Vec<u64>>'),
            SOURCE.replace('operation_id: String', '(operation_id, other): (String, String)'),
        ]:
            with self.subTest(source=source):
                with self.assertRaises(ValueError):
                    contracts.command_signatures(source)
        snake = contracts.command_signatures(SOURCE.replace('#[tauri::command]', '#[tauri::command(rename_all="snake_case")]'))
        self.assertEqual(snake['inspect']['arguments'][0]['wireName'], 'operation_id')

    def test_repository_registry_matches_all_live_handler_signatures(self):
        commands = contracts.registry_commands(ROOT)
        self.assertTrue(commands)
        self.assertTrue(all('signature' in command for command in commands))
        self.assertTrue(all(command['signature']['response']['ok'] for command in commands))
        for path, generated in contracts.products(ROOT).items():
            self.assertEqual(path.read_text(), generated, str(path.relative_to(ROOT)))


if __name__ == '__main__':
    unittest.main()
