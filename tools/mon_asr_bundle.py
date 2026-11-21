"""Assemble the small app-built transducer runtime independently of model weights."""
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import tarfile
# Support both direct CLI execution and file-based release/test imports.
import importlib.util
_catalog_spec = importlib.util.spec_from_file_location("mon_model_catalog", Path(__file__).with_name("mon_model_catalog.py"))
mon_model_catalog = importlib.util.module_from_spec(_catalog_spec)
_catalog_spec.loader.exec_module(mon_model_catalog)

def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()
def assemble(package):
    package = Path(package)
    name = 'mon-transducer-worker' + ('.exe' if (package/'mon-transducer-worker.exe').exists() else '')
    files = [(package/name, 'bin/'+name, 'executable'),
             (package/'nlohmann-MIT.txt', 'notices/nlohmann-MIT.txt', 'notice')]
    archive = package/'asr-runtime.tar'
    with tarfile.open(archive, 'w', format=tarfile.USTAR_FORMAT) as output:
        for source, relative, _ in files:
            data=source.read_bytes();entry=tarfile.TarInfo(relative);entry.size=len(data);entry.mode=0o600;entry.mtime=0
            output.addfile(entry,io.BytesIO(data))
    index=dict(schemaVersion=1,archiveBytes=archive.stat().st_size,archiveSha256=digest(archive),
               files=[dict(relativePath=relative,role=role,expectedSize=source.stat().st_size,expectedHash=digest(source)) for source,relative,role in files])
    (package/'asr-index.json').write_text(json.dumps(index,indent=2)+'\n',encoding='utf8',newline='\n')
    return index

def append_windows_choices(package):
    """Add the same ASR models using signed Windows worker/DLL package bytes."""
    import copy
    import shutil
    package=Path(package)
    asr=package/'asr'
    worker=asr/'bin/mon-transducer-worker.exe'
    notice=asr/'notices/nlohmann-MIT.txt'
    worker.parent.mkdir(parents=True,exist_ok=True)
    notice.parent.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(package/'workers/mon-transducer-worker.exe',worker)
    shutil.copyfile(package/'workers/nlohmann-MIT.txt',notice)
    added=[(worker,'bin/mon-transducer-worker.exe','executable'),
           (notice,'notices/nlohmann-MIT.txt','notice')]
    native_files=[]
    target=package/'runtime-asr.pending.tar'
    with tarfile.open(package/'runtime.tar') as original,tarfile.open(target,'w',format=tarfile.USTAR_FORMAT) as output:
        for member in original:
            if not member.isfile() or member.name.startswith('asr/'):continue
            with original.extractfile(member) as stream:
                output.addfile(member,stream)
                path=PurePosixPath(member.name)
                if path.parent==PurePosixPath('sherpa') and path.suffix.lower()=='.dll':
                    # ASR installs its DLLs under lib/ while diarization uses the
                    # runtime root. Both recipes extract the same signed member.
                    stream.seek(0)
                    file=dict(relativePath=f'lib/{path.name}',role='runtime_library',expectedSize=member.size,
                              expectedHash=hashlib.file_digest(stream,'sha256').hexdigest())
                    native_files.append((file,member.name))
        if not any(member=='sherpa/sherpa-onnx-c-api.dll' for _,member in native_files):
            raise ValueError('Windows sherpa C API missing')
        for source,relative,role in added:
            entry=tarfile.TarInfo(f'asr/{relative}');entry.size=source.stat().st_size;entry.mode=0o600;entry.mtime=0
            with source.open('rb') as stream:
                output.addfile(entry,stream)
            file=dict(relativePath=relative,role=role,expectedSize=entry.size,expectedHash=digest(source))
            native_files.append((file,entry.name))
    target.replace(package/'runtime.tar')
    archive=dict(format='tar',expectedSize=(package/'runtime.tar').stat().st_size,expectedHash=digest(package/'runtime.tar'))
    windows_uri='mon-bundled://windows-runtime-v1/runtime.tar'
    native_files.sort(key=lambda item:item[0]['relativePath'])
    files=[file for file,_ in native_files]
    sources=[dict(relativePath=file['relativePath'],url=windows_uri,archive=dict(archive,memberPath=member))
             for file,member in native_files]
    choices=json.loads((package/'catalog.json').read_text(encoding='utf8'))
    choices=[c for c in choices if c['recipe']['manifest']['engine']!='asr']
    for choice in choices:
        for source in choice['recipe']['sources']:
            if source['url']==windows_uri:source['archive'].update(archive)
    for template in mon_model_catalog.compile_target('windows_x86_64', 'asr'):
        choice=copy.deepcopy(template);recipe=choice['recipe'];manifest=recipe['manifest']
        pairs=[(f,s) for f,s in zip(manifest['files'],recipe['sources']) if f['role']!='runtime_library']
        manifest['files']=files+[f for f,s in pairs];recipe['sources']=sources+[s for f,s in pairs]
        manifest['backend']['files']['library']='lib/sherpa-onnx-c-api.dll'
        choices.append(choice)
    (package/'catalog.json').write_text(json.dumps(choices,ensure_ascii=False,indent=2)+'\n',encoding='utf8',newline='\n')
