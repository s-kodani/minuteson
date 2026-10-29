#!/usr/bin/env python3
"""Compile neutral model definitions with platform runtime descriptions.

Signed Windows runtime bytes are added by its existing packaging step. Generated
Mac catalogs remain compatibility artifacts, never the Windows model source.
"""
import argparse
import copy
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / 'app/model-catalog'


def definitions():
    return json.loads((CATALOG / 'model-definitions.json').read_text(encoding='utf8'))


def compile_target(target, group='base'):
    platforms = json.loads((CATALOG / 'platform-runtimes.json').read_text(encoding='utf8'))
    if platforms.get('schemaVersion') != 1 or target not in platforms['targets']:
        raise ValueError('unsupported platform runtime')
    platform = platforms['targets'][target]
    choices = []
    seen = set()
    for definition in definitions():
        if definition['group'] != group or target not in definition['targets']:
            continue
        if definition['id'] in seen:
            raise ValueError('duplicate model definition')
        seen.add(definition['id'])
        runtime = platform['choices'].get(definition['id'], {})
        manifest = copy.deepcopy(definition['manifest'])
        manifest['target'] = target
        files = copy.deepcopy(definition['files'] + runtime.get('files', []))
        sources = copy.deepcopy(definition['sources'] + runtime.get('sources', []))
        if len(files) != len(sources):
            raise ValueError('model files and sources disagree')
        pairs = {}
        for file, source in zip(files, sources):
            name = file['relativePath']
            if name != source['relativePath'] or name.casefold() in pairs:
                raise ValueError('duplicate or mismatched model member')
            pairs[name.casefold()] = (file, source)
        order = runtime.get('order', [file['relativePath'] for file in files])
        if len(order) != len(pairs) or len({name.casefold() for name in order}) != len(pairs):
            raise ValueError('runtime member ordering disagrees')
        ordered = [pairs[name.casefold()] for name in order]
        manifest['files'] = [file for file, _ in ordered]
        if 'backend' in manifest:
            manifest['backend']['files'].update(runtime.get('backendFiles', {}))
            manifest['backend']['files'].update(platform.get('backendFiles', {}).get(manifest['engine'], {}))
        choice = {key: copy.deepcopy(definition[key]) for key in ('id', 'label', 'description')}
        choice['id'] = platform['idPrefix'] + choice['id']
        if group == 'base':
            choice['label'] += platform['labelSuffix'].get(manifest['engine'], '')
        if 'tokenLimits' in definition:
            choice['tokenLimits'] = copy.deepcopy(definition['tokenLimits'])
        choice['recipe'] = dict(schemaVersion=definition['recipeSchemaVersion'], manifest=manifest,
                                sources=[source for _, source in ordered])
        choices.append(choice)
    return choices


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for group, name in [('base', 'macos.json'), ('asr', 'asr-transducer.json')]:
        choices = compile_target('macos_aarch64', group)
        destination = CATALOG / name
        if args.check:
            if json.loads(destination.read_text(encoding='utf8')) != choices:
                raise SystemExit(f'{destination}: regenerate with tools/mon_model_catalog.py')
        else:
            destination.write_text(json.dumps(choices, ensure_ascii=False, indent=2) + '\n', encoding='utf8')


if __name__ == '__main__':
    main()
