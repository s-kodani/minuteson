import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools'))
import mon_windows_parity_assets as assets


class RepresentativeAssetIsolationTests(unittest.TestCase):
    def test_private_verification_assets_do_not_enter_the_shared_ci_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);shared=root/'public-cache';private=root/'verification-only'
            data=b'synthetic weights';digest=hashlib.sha256(data).hexdigest()
            (private/'assets').mkdir(parents=True);(private/'assets'/digest).write_bytes(data)
            (private/'download-ledger.json').write_text(json.dumps(dict(downloadedBytes=7,attempts=[])))
            with patch.object(assets,'BASE',shared),patch.object(assets,'LEDGER',shared/'download-ledger.json'):
                fetch=assets.Fetch(8,base=private)
                self.assertEqual(fetch.acquire('https://example.invalid/model',len(data),digest),private/'assets'/digest)
                self.assertEqual(fetch.ledger['downloadedBytes'],7)
                self.assertFalse(shared.exists())
                with self.assertRaisesRegex(ValueError,'budget'):assets.Fetch(6,base=private)

    def test_native_validation_reports_only_json_and_keeps_weights_out_of_cache_paths(self):
        import yaml
        workflow=yaml.safe_load((ROOT/'.github/workflows/desktop-platform-build.yml').read_text())
        steps=workflow['jobs']['native']['steps']
        for step in steps:
            if 'actions/cache/' in step.get('uses','') or 'actions/upload-artifact' in step.get('uses',''):
                self.assertNotIn('.cache/llama-runtime-check',step.get('with',{}).get('path',''))
        check=next(step for step in steps if step.get('name')=='Validate upgraded LLM runtimes with representative weights')
        self.assertEqual(check['if'],"env.MON_CI_FULL == 'true'")
        self.assertIn('--report output/rust-tests/llama-runtime-',check['run'])
