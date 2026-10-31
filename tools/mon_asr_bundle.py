"""Assemble the small app-built transducer runtime independently of model weights."""
import hashlib
import io
import json
from pathlib import Path
import tarfile
# Support both direct CLI execution and file-based release/test imports.
import importlib.util
_catalog_spec = importlib.util.spec_from_file_location("mon_model_catalog", Path(__file__).with_name("mon_model_catalog.py"))
mon_model_catalog = importlib.util.module_from_spec(_catalog_spec)
_catalog_spec.loader.exec_module(mon_model_catalog)

URI = 'mon-bundled://asr-v1/asr-runtime.tar'
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
def asset_members():
    """Read only explicitly declared, checksum-pinned model-specific assets."""
    root = Path(__file__).resolve().parents[1] / 'third-party/asr-assets'
    members = []
    for definition in mon_model_catalog.definitions():
        if definition['group'] != 'asr':
            continue
        for file, source in zip(definition['files'], definition['sources']):
            if source['url'] != URI:
                continue
            relative = file['relativePath']
            member = f"assets/{definition['id']}/{relative}"
            if (file['role'] != 'asr_asset' or source['relativePath'] != relative
                    or source.get('archive', {}).get('memberPath') != member):
                raise ValueError('invalid declared ASR model asset')
            path = root / definition['id'] / relative
            if not path.resolve().is_relative_to(root.resolve()):
                raise ValueError('ASR model asset path escaped the source tree')
            if path.stat().st_size != file['expectedSize'] or digest(path) != file['expectedHash']:
                raise ValueError('ASR model asset differs from its catalog SHA-256')
            members.append((definition['id'], file, path, member))
    return members

def assemble(package):
    package = Path(package)
    name = 'mon-transducer-worker' + ('.exe' if (package/'mon-transducer-worker.exe').exists() else '')
    files = [(package/name, 'bin/'+name, 'executable'),
             (package/'nlohmann-MIT.txt', 'notices/nlohmann-MIT.txt', 'notice')]
    archive = package/'asr-runtime.tar'
    assets = asset_members()
    with tarfile.open(archive, 'w', format=tarfile.USTAR_FORMAT) as output:
        for source, relative in [(source, relative) for source, relative, _ in files] + [(source, member) for _, _, source, member in assets]:
            data=source.read_bytes();entry=tarfile.TarInfo(relative);entry.size=len(data);entry.mode=0o600;entry.mtime=0
            output.addfile(entry,io.BytesIO(data))
    index=dict(schemaVersion=1,archiveBytes=archive.stat().st_size,archiveSha256=digest(archive),
               files=[dict(relativePath=relative,role=role,expectedSize=source.stat().st_size,expectedHash=digest(source)) for source,relative,role in files])
    if assets:
        index['modelAssets'] = {}
        for identifier, file, _, _ in assets:
            index['modelAssets'].setdefault(identifier, []).append(file)
    (package/'asr-index.json').write_text(json.dumps(index,indent=2)+'\n',encoding='utf8',newline='\n')
    return index

def append_windows_choices(package):
    """Add the same ASR models using signed Windows worker/DLL package bytes."""
    import copy
    import shutil
    package=Path(package)
    root=Path(__file__).resolve().parents[1]
    asr=package/'asr'
    (asr/'bin').mkdir(parents=True,exist_ok=True)
    (asr/'lib').mkdir(exist_ok=True)
    (asr/'notices').mkdir(exist_ok=True)
    shutil.copyfile(package/'workers/mon-transducer-worker.exe',asr/'bin/mon-transducer-worker.exe')
    shutil.copyfile(package/'workers/nlohmann-MIT.txt',asr/'notices/nlohmann-MIT.txt')
    for source in (package/'sherpa').glob('*.dll'):
        shutil.copyfile(source,asr/'lib'/source.name)
    assets = asset_members()
    for _, _, source, member in assets:
        destination = asr / member
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    # Assets belong to the declaring model, not every model in the archive.
    common = sorted(path for directory in ('bin', 'lib', 'notices')
                    for path in (asr/directory).rglob('*') if path.is_file())
    archive_files = sorted(common + [asr/member for _, _, _, member in assets])
    target=package/'runtime-asr.pending.tar'
    with tarfile.open(package/'runtime.tar') as original,tarfile.open(target,'w',format=tarfile.USTAR_FORMAT) as output:
        for member in original:
            if not member.isfile() or member.name.startswith('asr/'):continue
            output.addfile(member,original.extractfile(member))
        for source in archive_files:
            if source.is_file():
                data=source.read_bytes();entry=tarfile.TarInfo(source.relative_to(package).as_posix());entry.size=len(data);entry.mode=0o600;entry.mtime=0
                output.addfile(entry,io.BytesIO(data))
    target.replace(package/'runtime.tar')
    archive=dict(format='tar',expectedSize=(package/'runtime.tar').stat().st_size,expectedHash=digest(package/'runtime.tar'))
    windows_uri='mon-bundled://windows-runtime-v1/runtime.tar'
    choices=json.loads((package/'catalog.json').read_text(encoding='utf8'))
    choices=[c for c in choices if c['recipe']['manifest']['engine']!='asr']
    for choice in choices:
        for source in choice['recipe']['sources']:
            if source['url']==windows_uri:source['archive'].update(archive)
    for template in mon_model_catalog.compile_target('windows_x86_64', 'asr'):
        choice=copy.deepcopy(template);recipe=choice['recipe'];manifest=recipe['manifest']
        pairs=[(f,s) for f,s in zip(manifest['files'],recipe['sources']) if f['role']!='runtime_library']
        files=[];sources=[]
        for path in common:
            if not path.is_file():continue
            relative=path.relative_to(asr).as_posix()
            files.append(dict(relativePath=relative,role='executable' if path.suffix=='.exe' else 'runtime_library' if path.suffix=='.dll' else 'notice',expectedSize=path.stat().st_size,expectedHash=digest(path)))
            sources.append(dict(relativePath=relative,url=windows_uri,archive=dict(archive,memberPath='asr/'+relative)))
        for file, source in pairs:
            if source['url'] == URI:
                identifier = choice['id'].removeprefix('windows-')
                member = f"assets/{identifier}/{file['relativePath']}"
                if not any(identifier == candidate and file == declared and member == actual
                           for candidate, declared, _, actual in assets):
                    raise ValueError('Windows ASR model asset is not declared')
                source['url'] = windows_uri
                source['archive'] = dict(archive, memberPath='asr/'+member)
        manifest['files']=files+[f for f,s in pairs];recipe['sources']=sources+[s for f,s in pairs]
        manifest['backend']['files']['library']='lib/sherpa-onnx-c-api.dll'
        if not (asr/manifest['backend']['files']['library']).is_file():raise ValueError('Windows sherpa C API missing')
        choices.append(choice)
    (package/'catalog.json').write_text(json.dumps(choices,ensure_ascii=False,indent=2)+'\n',encoding='utf8',newline='\n')
