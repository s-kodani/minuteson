"""Build/package pinned Windows CPU inference engines and download choices.

No model weights are bundled. Upstream archives are checksum checked before
reading members; the generated runtime index binds every packaged byte.
"""
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request
import zipfile
# Support both direct CLI execution and file-based release/test imports.
import importlib.util
_catalog_spec = importlib.util.spec_from_file_location("mon_model_catalog", Path(__file__).with_name("mon_model_catalog.py"))
mon_model_catalog = importlib.util.module_from_spec(_catalog_spec)
_catalog_spec.loader.exec_module(mon_model_catalog)

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / '.cache/windows-runtime'
PACKAGE = CACHE / 'package'
URI = 'mon-bundled://windows-runtime-v1/runtime.tar'
LLAMA = ('https://github.com/ggml-org/llama.cpp/releases/download/b11533/llama-b11533-bin-win-cpu-x64.zip', 19513403, '659ac73690e588bf60ba1c8de02479683e60ed2ce2915ddf3e15904b520fa933')
LLAMA_11960 = ('https://github.com/ggml-org/llama.cpp/releases/download/b11960/llama-b11960-bin-win-cpu-x64.zip', 24407745, '73f962f82850d244db47eaacb9d913637027b82399124324298576d58c5f4287')
SHERPA = ('https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts.tar.bz2',23271851,'4b0a94f7b5c606b1b64a19a831c2127559e4b3d34e195465ebc7be73d9ed4783')

def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()

def sign(path):
    if os.environ.get('MON_WINDOWS_RELEASE_SIGNING')=='1':
        subprocess.run(['powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',
            str(ROOT/'tools/sign-windows.ps1'),'-Path',str(path)],check=True)

def acquire(url,size,sha):
    target=CACHE/'downloads'/(sha[:16]+'-'+url.rsplit('/',1)[-1])
    target.parent.mkdir(parents=True,exist_ok=True)
    if not target.exists():
        from mon_windows_parity_assets import Fetch
        return Fetch(30*1024**3).acquire(url,size,sha)
    if target.stat().st_size!=size or digest(target)!=sha: raise ValueError('cached upstream differs')
    return target

def dump(data,path):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_bytes(data)

def build_whisper():
    definition=json.loads((ROOT/'app/model-catalog/whisper-runtime-build.json').read_text(encoding="utf8"))
    archive=acquire(definition['sourceUrl'],definition['sourceBytes'],definition['sourceSha256'])
    source=CACHE/'whisper-source'
    if not source.exists():
        with tarfile.open(archive) as tar:
            if any(not (m.isdir() or m.isfile()) or '..' in Path(m.name).parts or Path(m.name).is_absolute() for m in tar.getmembers()):
                raise ValueError('unsafe source archive')
            tar.extractall(source,filter='data')
    source=next(source.iterdir())
    build=CACHE/'whisper-build'
    options=['-DBUILD_SHARED_LIBS=OFF','-DGGML_NATIVE=OFF','-DGGML_OPENMP=OFF','-DGGML_VULKAN=OFF','-DGGML_BACKEND_DL=OFF','-DWHISPER_BUILD_TESTS=OFF','-DWHISPER_BUILD_EXAMPLES=ON','-DGGML_AVX=OFF','-DGGML_AVX2=OFF','-DGGML_FMA=OFF','-DGGML_F16C=OFF','-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded']
    subprocess.run(['cmake','-S',str(source),'-B',str(build),'-G','Visual Studio 17 2022','-A','x64',*options],check=True)
    subprocess.run(['cmake','--build',str(build),'--config','Release','--target','whisper-cli','--parallel','4'],check=True)
    dump((build/'bin/Release/whisper-cli.exe').read_bytes(),PACKAGE/'whisper/whisper-cli.exe')
    dump((source/'LICENSE').read_bytes(),PACKAGE/'whisper/notices/whisper-MIT.txt')

def upstream_engines():
    for engine, release in [('llama', LLAMA), ('llama11960', LLAMA_11960)]:
        with zipfile.ZipFile(acquire(*release)) as archive:
            for name in archive.namelist():
                leaf=Path(name).name
                if leaf=='llama-completion.exe' or leaf.lower().endswith('.dll') or leaf=='LICENSE':
                    if not leaf: continue
                    rel='notices/llama-MIT.txt' if leaf=='LICENSE' else leaf
                    dump(archive.read(name),PACKAGE/engine/rel)
    with tarfile.open(acquire(*SHERPA)) as archive:
        for member in archive.getmembers():
            leaf=Path(member.name).name
            if member.isfile() and (leaf=='sherpa-onnx-offline-speaker-diarization.exe' or leaf.lower().endswith('.dll')):
                dump(archive.extractfile(member).read(),PACKAGE/'sherpa'/leaf)
    # Ship engine notices with the app itself, before any model is downloaded.
    upstream=mon_model_catalog.compile_target('windows_x86_64')
    for choice in upstream:
        if choice['id'] not in ('llama-b11533-qwen25-15b-q4km','sherpa-1138-pyannote3-eres2net-base'):continue
        engine='llama' if choice['recipe']['manifest']['engine']=='llama.cpp' else 'sherpa'
        for file,source in zip(choice['recipe']['manifest']['files'],choice['recipe']['sources']):
            if file['role']!='notice':continue
            if file['relativePath']=='notices/Qwen-Apache-2.0.txt' or file['relativePath']=='notices/segmentation-MIT.txt':continue
            url=source['url']
            if file['relativePath']=='notices/llama-MIT.txt':url='https://raw.githubusercontent.com/ggml-org/llama.cpp/edd6e2bbdad5930899a93db8fa73c3b61c7b9bcc/LICENSE'
            path=acquire(url,file['expectedSize'],file['expectedHash'])
            dump(path.read_bytes(),PACKAGE/engine/file['relativePath'])
    # The CPU package is verified as an entire upstream release. Preserve the
    # full DLL dispatch set rather than depending on the build host's PATH.
    print('Windows runtime files:',[str(p.relative_to(PACKAGE)) for p in PACKAGE.rglob('*') if p.is_file()],flush=True)

def catalog():
    upstream=mon_model_catalog.compile_target('windows_x86_64')
    whisper_network=json.loads((ROOT/'app/model-catalog/whisper-small-network.json').read_text(encoding="utf8"))
    variants=json.loads((ROOT/'app/model-catalog/whisper-variants.json').read_text(encoding="utf8"))
    choices=[]
    for variant in variants:
        network=copy.deepcopy(whisper_network)
        model_path=f"models/ggml-{variant['name']}.bin"
        network['files'][0].update(relativePath=model_path,expectedSize=variant['bytes'],expectedHash=variant['sha256'])
        network['sources'][0].update(relativePath=model_path,url=f"https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-{variant['name']}.bin")
        vad=json.loads((ROOT/'app/model-catalog/whisper-vad-network.json').read_text(encoding="utf8"))
        network['files']+=vad['files'];network['sources']+=vad['sources']
        choices.append(dict(id=f"windows-whisper-194-{variant['name']}",label=f"Whisper {variant['name']}（多言語・CPU／Vulkan）",description='Windows x64でGPUを試行し、利用できない場合はCPUで文字起こしします。取得後に動作確認して適用します。',recipe=dict(schemaVersion=4,manifest=dict(schemaVersion=1,engine='whisper.cpp',engineVersion='v1.9.4',modelName=f"Whisper {variant['name']}",modelVersion='5359861c739e955e79d9a303bcbc70fb988958b1',quantization=None,licenseRef='whisper.cpp / OpenAI Whisper MIT. Notices included. Windows CPU build; quality not evaluated.',target='windows_x86_64',files=network['files']),sources=network['sources'])))
    for choice in upstream:
        if choice['recipe']['manifest']['engine'] not in ('llama.cpp','sherpa-onnx'):
            raise ValueError('unsupported shared model engine')
        choices.append(choice)
    executables={'whisper':'whisper-cli.exe','llama':'llama-completion.exe','llama11960':'llama-completion.exe','sherpa':'sherpa-onnx-offline-speaker-diarization.exe'}
    engine_ids={'whisper.cpp':'whisper','llama.cpp':'llama','sherpa-onnx':'sherpa'}
    # App-local VC runtime for upstream llama DLLs: do not depend on a developer
    # machine's global PATH or preinstalled redistributable.
    redists=list(Path(os.environ.get('ProgramFiles(x86)','C:/Program Files (x86)')).glob('Microsoft Visual Studio/2022/*/VC/Redist/MSVC/*/x64/Microsoft.VC143.CRT'))
    if not redists: raise ValueError('Visual C++ redistributable directory missing')
    for name in ('vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll'):
        for engine in ('llama', 'llama11960'):
            dump((sorted(redists)[-1]/name).read_bytes(),PACKAGE/engine/name)
    # Sign before generating hashes: installed bytes must match the recipes.
    for engine in executables:
        for path in sorted((PACKAGE/engine).rglob('*')):
            if path.is_file() and path.suffix.lower() in ('.exe','.dll'):sign(path)
    def native_files(engine):
        paths=sorted(p for p in (PACKAGE/engine).rglob('*') if p.is_file())
        if engine in ('llama', 'llama11960'):
            keep={'llama-completion.exe','llama-completion-impl.dll','llama-common.dll','llama.dll','ggml.dll','ggml-base.dll','libomp.dll','ggml-cpu-x64.dll','ggml-cpu-haswell.dll','ggml-cpu-skylakex.dll','ggml-cpu-zen4.dll','ggml-cpu-sapphirerapids.dll','vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll'}
            paths=[p for p in paths if p.name in keep or p.relative_to(PACKAGE/engine).parts[0]=='notices']
        return paths
    # Create one deterministic TAR of native bytes. Runtime filenames and their
    # compiled hashes are fixed in each recipe generated for this very bundle.
    with tarfile.open(PACKAGE/'runtime.tar','w',format=tarfile.USTAR_FORMAT) as archive:
        for engine in executables:
            for path in native_files(engine):
                if path.is_file():
                    data=path.read_bytes();info=tarfile.TarInfo(path.relative_to(PACKAGE).as_posix());info.size=len(data);info.mode=0o600;info.mtime=0
                    archive.addfile(info,io.BytesIO(data))
    archive=dict(format='tar',expectedSize=(PACKAGE/'runtime.tar').stat().st_size,expectedHash=digest(PACKAGE/'runtime.tar'))
    snapshots=[]
    for engine,name in executables.items():
        path=PACKAGE/engine/name
        if not path.is_file(): raise ValueError(f'missing native engine {path}')
        snapshots.append(dict(protocol=2,engine=engine,name=name,bytes=path.stat().st_size,sha256=digest(path)))
    for choice in choices:
        recipe=choice['recipe'];engine=engine_ids[recipe['manifest']['engine']]
        if engine=='llama' and recipe['manifest']['engineVersion']=='b11960':engine='llama11960'
        native=[];sources=[]
        for path in native_files(engine):
            if not path.is_file():continue
            rel=path.relative_to(PACKAGE/engine).as_posix()
            native.append(dict(relativePath=rel,role='executable' if rel==executables[engine] else 'runtime_library' if rel.endswith('.dll') else 'notice',expectedSize=path.stat().st_size,expectedHash=digest(path)))
            sources.append(dict(relativePath=rel,url=URI,archive=dict(archive,memberPath=path.relative_to(PACKAGE).as_posix())))
        if sum(f['role']=='runtime_library' for f in native)>16:raise ValueError('too many native DLLs for the package contract')
        existing=[(f,s) for f,s in zip(recipe['manifest']['files'],recipe['sources']) if f['relativePath'] not in {n['relativePath'] for n in native}]
        recipe['manifest']['files']=native+[f for f,s in existing];recipe['sources']=sources+[s for f,s in existing]
    (PACKAGE/'catalog.json').write_text(json.dumps(choices,ensure_ascii=False,indent=2)+'\n',encoding='utf8',newline='\n')
    (PACKAGE/'runtime-index.json').write_text(json.dumps(snapshots,indent=2)+'\n',encoding='utf8',newline='\n')

def build_embedding():
    helper=PACKAGE/'mon-speaker-embedding.exe'
    subprocess.run(['rustc','--edition=2021','-C','opt-level=2','-C','panic=abort',
                    str(ROOT/'probes/speaker-embedding/main.rs'),'-o',str(helper)],check=True)
    sign(helper)
    (PACKAGE/'embedding-index.json').write_text(json.dumps(dict(schemaVersion=1,name=helper.name,
        bytes=helper.stat().st_size,sha256=digest(helper)))+'\n',encoding='utf8',newline='\n')

def main():
    if os.name!='nt':raise ValueError('build Windows native runtimes on Windows')
    PACKAGE.mkdir(parents=True,exist_ok=True)
    build_whisper();upstream_engines();catalog();build_embedding()
    import mon_windows_workers
    mon_windows_workers.prepare()
    mon_windows_workers.prepare(vulkan=True)
    from mon_asr_bundle import append_windows_choices
    append_windows_choices(PACKAGE)
    print('Windows native runtime package ready:',PACKAGE)

if __name__=='__main__':main()
