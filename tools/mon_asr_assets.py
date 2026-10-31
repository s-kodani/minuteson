#!/usr/bin/env python3
"""Reproduce the small, pinned Haas hotword vocabulary without model weights.

The conversion follows sherpa-onnx's equal-score/longest-match example. It is
only a hotword encoder; it does not change the ASR model's output token IDs.
"""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSET_ID = 'haas-ja-v1.0-onnx-quantized'
DIRECTORY = ROOT / 'third-party/asr-assets' / ASSET_ID
REVISION = '7f5ab0a4b1bf39db78afd6691eeecbc4f9d42e39'
SOURCE_SHA = 'ec78b13c5d9f156cf0d5fdbd6b5aec5785680f319c92630c11d5d11b14b83a84'


def vocabulary(tokens):
    if hashlib.sha256(tokens).hexdigest() != SOURCE_SHA:
        raise ValueError('Haas tokens do not match the pinned source')
    rows = [row.rsplit(' ', 1) for row in tokens.decode('utf8').splitlines() if row]
    if len(rows) != 3073 or any(len(row) != 2 or int(row[1]) != i for i, row in enumerate(rows)):
        raise ValueError('Haas token IDs are not the complete ordered vocabulary')
    if rows[0][0] != '<unk>' or rows[-1][0] != '<blk>':
        raise ValueError('Haas special token IDs changed')
    return ''.join(f'{piece}\t-1.0\n' for piece, _ in rows).encode('utf8')


def outputs():
    tokens = (DIRECTORY / 'source/tokens.txt').read_bytes()
    data = vocabulary(tokens)
    provenance = {
        'schemaVersion': 1,
        'source': f'https://huggingface.co/oboroge0/Haas-ja-v1.0-onnx-quantized/resolve/{REVISION}/tokens.txt',
        'sourceBytes': len(tokens),
        'sourceSha256': SOURCE_SHA,
        'generator': 'tools/mon_asr_assets.py',
        'algorithm': 'sherpa-equal-score-hotword-vocabulary-v1',
        'reference': 'https://github.com/k2-fsa/sherpa-onnx/blob/11afbd009a7f8c08f4bcf2fc1b265d0df4670fbf/nodejs-addon-examples/test_asr_non_streaming_nemo_parakeet_tdt_v2_hotwords.js',
        'description': 'Each token piece in ID order receives score -1.0. This yields a longest-match hotword encoder; it is not the original SentencePiece model.',
        'output': 'models/bpe.vocab',
        'outputBytes': len(data),
        'outputSha256': hashlib.sha256(data).hexdigest(),
        'license': 'Apache-2.0; see LICENSE and NOTICE in this directory',
    }
    return {'models/bpe.vocab': data,
            'provenance.json': (json.dumps(provenance, ensure_ascii=False, indent=2) + '\n').encode('utf8')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for relative, data in outputs().items():
        path = DIRECTORY / relative
        if args.check:
            if path.read_bytes() != data:
                raise SystemExit(f'{path}: regenerate with tools/mon_asr_assets.py')
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)


if __name__ == '__main__':
    main()
