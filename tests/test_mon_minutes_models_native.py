"""Opt-in macOS CPU smoke tests for the three additional minutes models.

MON_MINUTES_MODEL_DIR contains gemma.gguf, phi.gguf and lfm.gguf.
MON_RESIDENT_RUNTIME points to the built worker package. Only synthetic text
is used; no application settings or user meetings are opened.
"""
import json
import os
from pathlib import Path
import re
import tempfile
import unittest

from test_mon_inference_runtime_native import Worker


@unittest.skipUnless(os.environ.get('MON_MINUTES_MODEL_DIR') and
                     os.environ.get('MON_RESIDENT_RUNTIME'), 'explicit GGUF models and worker required')
class MinutesModelsNative(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='mon-minutes-models-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.runtime = Path(os.environ['MON_RESIDENT_RUNTIME']).resolve()
        self.worker_env = {'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'LANG': 'en_US.UTF-8'}

    def check_model(self, name, sampling):
        model = (Path(os.environ['MON_MINUTES_MODEL_DIR']) / (name + '.gguf')).resolve()
        worker = Worker(self, 'llama')
        for i in range(2):
            directory = self.root / str(i)
            directory.mkdir()
            for file in ('count.json', 'result.json', 'stderr.log'):
                (directory / file).touch()
            (directory / 'system.txt').write_text(
                'あなたは議事録を作成するアシスタントです。発言に基づいて日本語で要約し、'
                'markdown文字列を持つJSONオブジェクトのみを返してください。', encoding='utf8')
            (directory / 'prompt.txt').write_text(
                '田中：次回の会議は11月6日です。佐藤：資料は11月5日までに佐藤が用意します。'
                '決定事項と担当者を簡潔にまとめてください。', encoding='utf8')
            (directory / 'schema.json').write_text(json.dumps(dict(
                type='object', properties=dict(markdown=dict(type='string')),
                required=['markdown'], additionalProperties=False)))
            args = ['--model', str(model), '--system-prompt-file', str(directory / 'system.txt'),
                    '--file', str(directory / 'prompt.txt'), '--conversation', '--jinja', '--no-escape',
                    '--reasoning', 'off', '--offline', *sampling]
            worker.run(i * 2 + 1, args, directory, 'count.json', kind='llama_tokenize')
            counted = json.loads((directory / 'count.json').read_text())
            self.assertGreater(counted['tokens'], counted['systemPrefixTokens'])
            count_log = (directory / 'stderr.log').read_text(errors='replace')
            self.assertNotIn('ggml_metal_device_init', count_log)
            self.assertNotIn('ggml_vulkan:', count_log)
            args += ['--json-schema-file', str(directory / 'schema.json'), '--ctx-size', '4096',
                     '--predict', '512', '--threads', '4', '--threads-batch', '4', '--gpu-layers', '0',
                     '--device', 'none', '--no-kv-offload', '--no-op-offload', '--seed', '42',
                     '--single-turn', '--no-display-prompt', '--no-context-shift', '--no-warmup',
                     '--fit', 'off', '--simple-io', '--color', 'off', '--log-colors', 'off',
                     '--log-verbosity', '4', '--verbose-prompt']
            worker.run(i * 2 + 2, args, directory, 'result.json', kind='llama')
            text = (directory / 'result.json').read_text().strip()
            result, end = json.JSONDecoder().raw_decode(text)
            self.assertIn(text[end:].strip(), ('', '[end of text]'))
            self.assertEqual(set(result), {'markdown'})
            self.assertIn('佐藤', result['markdown'])
            self.assertIn('11月5日', result['markdown'])
            self.assertIn('11月6日', result['markdown'])
            self.assertNotIn('<think>', result['markdown'])
            log = (directory / 'stderr.log').read_text(errors='replace')
            measured = re.search(r'number of tokens in prompt = (\d+)', log)
            self.assertIsNotNone(measured)
            self.assertEqual(counted['tokens'], int(measured[1]))
            if os.environ.get('MON_MINUTES_NATIVE_OUTPUT'):
                target = Path(os.environ['MON_MINUTES_NATIVE_OUTPUT'])
                target.mkdir(parents=True, exist_ok=True)
                (target / f'{name}-{i}.json').write_text(json.dumps(
                    dict(model=name, tokens=counted['tokens'], result=result), ensure_ascii=False, indent=2) + '\n')
        self.assertEqual(sum((self.root / str(i) / 'stderr.log').read_text(errors='replace').count(
            'mon_model_load: llama=1') for i in range(2)), 1)
        # The production planner can count prompts beyond the former 64 KiB
        # worker limit. This does not allocate a long generation KV cache.
        directory = self.root / 'long-count'
        directory.mkdir()
        for file in ('count.json', 'stderr.log'):
            (directory / file).touch()
        (directory / 'system.txt').write_text('会議内容を要約してください。', encoding='utf8')
        (directory / 'prompt.txt').write_text('担当者と締切を確認しました。' * 3000, encoding='utf8')
        worker.run(5, ['--model', str(model), '--system-prompt-file', str(directory / 'system.txt'),
                      '--file', str(directory / 'prompt.txt'), '--conversation', '--jinja',
                      '--no-escape', '--reasoning', 'off', '--offline', *sampling],
                   directory, 'count.json', kind='llama_tokenize')
        self.assertGreater(json.loads((directory / 'count.json').read_text())['tokens'], counted['tokens'])
        worker.close()

    def test_gemma(self):
        self.check_model('gemma', ['--temp', '1.0', '--top-p', '0.95', '--top-k', '64', '--min-p', '0'])

    def test_phi(self):
        self.check_model('phi', ['--temp', '0'])

    def test_lfm(self):
        self.check_model('lfm', ['--temp', '0.1', '--top-k', '50', '--repeat-penalty', '1.1'])


if __name__ == '__main__':
    unittest.main()
