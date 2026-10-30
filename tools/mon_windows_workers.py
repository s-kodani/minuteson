"""Build Windows resident v1 workers from the same fixed engine sources as Mac.
The old protocol-2 CLI package remains available for existing saved jobs.
Additional source bytes share the parity download budget and SHA ledger.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import mon_inference_runtime as shared
import mon_windows_runtime as runtime
from mon_windows_parity_assets import Fetch

ROOT=Path(__file__).resolve().parents[1]
CACHE=ROOT/'.cache/windows-runtime'
OPTIONS=['-DBUILD_SHARED_LIBS=OFF','-DGGML_NATIVE=OFF','-DGGML_OPENMP=OFF','-DGGML_BACKEND_DL=OFF',
    '-DGGML_AVX=OFF','-DGGML_AVX2=OFF','-DGGML_FMA=OFF','-DGGML_F16C=OFF',
    '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded','-DCMAKE_CXX_FLAGS=/utf-8 /EHsc']

def extract(archive,target):
    if not (target/'.complete').exists():
        with tarfile.open(archive) as source:
            members=source.getmembers()
            if len(members)>40000 or sum(m.size for m in members)>512*1024*1024:raise ValueError('source expansion exceeds limit')
            if any(not(m.isfile() or m.isdir()) or '..' in Path(m.name).parts or Path(m.name).is_absolute() for m in members):raise ValueError('unsafe source member')
            selected=[]
            for member in members:
                parts=Path(member.name).parts[1:]
                # Mobile applications are unrelated to the CLI worker and
                # exceed legacy Windows extraction path limits.
                if len(parts)>1 and parts[0]=='examples' and parts[1].startswith(('whisper.android','whisper.objc','whisper.swift','whisper.wasm')):continue
                # The disabled llama tools UI also contains paths over MAX_PATH.
                if parts[:2]==('tools','ui'):continue
                member.name=str(Path('tree',*parts))
                selected.append(member)
            source.extractall(target,members=selected,filter='data')
        (target/'.complete').write_text('verified source\n',encoding='utf8')
    return target/'tree'

def prepare(vulkan=False):
    if vulkan:
        from mon_windows_vulkan_sdk import prepare as sdk
        sdk_root=sdk();os.environ['VULKAN_SDK']=str(sdk_root)
        os.environ['PATH']=str(sdk_root/'Bin')+os.pathsep+os.environ['PATH']
    fetch=Fetch(30*1024**3)
    definition=json.loads((ROOT/'app/model-catalog/whisper-runtime-build.json').read_text(encoding='utf8'))
    whisper=fetch.acquire(definition['sourceUrl'],definition['sourceBytes'],definition['sourceSha256'])
    llama=fetch.acquire(f'https://codeload.github.com/ggml-org/llama.cpp/tar.gz/{shared.LLAMA_COMMIT}',shared.LLAMA_BYTES,shared.LLAMA_SHA)
    header=fetch.acquire(f'https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/{shared.SHERPA_COMMIT}/sherpa-onnx/c-api/c-api.h',167657,shared.HEADER_SHA)
    digest=hashlib.sha256(Path(__file__).read_bytes()+Path(shared.__file__).read_bytes())
    for path in sorted((ROOT/'probes/inference-worker').iterdir()):
        if path.is_file():digest.update(path.name.encode()+path.read_bytes())
    stamp=digest.hexdigest()[:16]
    source_stamp=hashlib.sha256(Path(shared.__file__).read_bytes()).hexdigest()[:12]
    roots={}
    for engine,archive in [('whisper',whisper),('llama',llama)]:
        source=extract(archive,CACHE/f'workers-source-{source_stamp}'/engine)
        if not (source/'mon/.patched').exists():
            shutil.copytree(ROOT/'probes/inference-worker',source/'mon',dirs_exist_ok=True)
            shutil.copyfile(ROOT/'.cache/windows-capture-headers/json.hpp',source/'mon/json.hpp')
            shutil.copyfile(header,source/'mon/c-api.h')
            (shared.patch_whisper if engine=='whisper' else shared.patch_llama)(source)
            (source/'mon/.patched').write_text(source_stamp,encoding='utf8')
        shutil.copytree(ROOT/'probes/inference-worker',source/'mon',dirs_exist_ok=True)
        cmake=source/'CMakeLists.txt'
        marker='# MON Windows UTF-8 and long native paths'
        text=cmake.read_text(encoding='utf8')
        if marker not in text:
            text+='\n'+marker+'\nif(WIN32)\n'
            for target in shared.NAMES.values():
                text+=f'  if(TARGET {target})\n    target_link_options({target} PRIVATE "/MANIFESTINPUT:${{CMAKE_CURRENT_SOURCE_DIR}}/mon/windows.manifest")\n  endif()\n'
            text+='endif()\n';cmake.write_text(text,encoding='utf8')
        roots[engine]=source
    package=runtime.PACKAGE/'workers';package.mkdir(parents=True,exist_ok=True)
    if vulkan:
        shutil.copyfile(sdk_root/'Licenses/LICENSE.txt',package/'Vulkan-SDK-LICENSE.txt')
        (package/'Vulkan-Headers-LICENSE.txt').write_bytes(b'Copyright 2014-2025 The Khronos Group Inc.\nVulkan SDK 1.4.328.1 headers (Apache-2.0).\n\n'+(ROOT/'third-party/notices/defmt-LICENSE-APACHE.txt').read_bytes())
    for engine,source in roots.items():
        build=(ROOT/'.cache'/('vk'+engine[0]+'-'+source_stamp[:6])) if vulkan else CACHE/f'workers-build-{source_stamp}'/engine
        options=OPTIONS+['-DGGML_VULKAN='+('ON' if vulkan else 'OFF')]
        if engine=='whisper':options+=['-DWHISPER_BUILD_TESTS=OFF','-DWHISPER_BUILD_EXAMPLES=ON']
        else:options+=['-DLLAMA_BUILD_COMMON=ON','-DLLAMA_BUILD_TESTS=OFF','-DLLAMA_BUILD_TOOLS=OFF','-DLLAMA_BUILD_EXAMPLES=OFF','-DLLAMA_BUILD_APP=OFF','-DLLAMA_BUILD_MTMD=OFF','-DLLAMA_OPENSSL=OFF']
        subprocess.run(['cmake','-S',str(source),'-B',str(build),'-G','Visual Studio 17 2022','-A','x64',*options],check=True)
        targets=[shared.NAMES[engine]]+([shared.NAMES['sherpa'], shared.NAMES['asr-transducer']] if engine=='llama' and not vulkan else [])
        subprocess.run(['cmake','--build',str(build),'--config','Release','--target',*targets,'--parallel','4'],check=True)
        for name in targets:
            candidates=list(build.rglob(name+'.exe'));candidates=[p for p in candidates if p.parent.name=='Release']
            if len(candidates)!=1:raise ValueError('worker output missing or ambiguous')
            output=name.replace('-worker','-vulkan-worker') if vulkan else name
            shutil.copyfile(candidates[0],package/(output+'.exe'));runtime.sign(package/(output+'.exe'))
    index=[dict(protocol=1,engine=engine,name=name+'.exe',bytes=(package/(name+'.exe')).stat().st_size,sha256=runtime.digest(package/(name+'.exe'))) for engine,name in shared.NAMES.items()]
    if vulkan:
        for item in index:
            if item['engine'] in ('sherpa','asr-transducer'):continue
            path=package/(shared.NAMES[item['engine']].replace('-worker','-vulkan-worker')+'.exe')
            item['gpu']=dict(name=path.name,bytes=path.stat().st_size,sha256=runtime.digest(path))
    (package/'runtime-index.json').write_text(json.dumps(index,indent=2)+'\n',encoding='utf8')
    (package/('build-info-vulkan.json' if vulkan else 'build-info.json')).write_text(json.dumps(dict(recipeSha256=stamp,whisperSource=definition['sourceSha256'],llamaCommit=shared.LLAMA_COMMIT,sherpaCommit=shared.SHERPA_COMMIT,vulkan=vulkan,protocol=1),indent=2)+'\n',encoding='utf8')
    for engine,source in roots.items():shutil.copyfile(source/'LICENSE',package/(engine+'-MIT.txt'))
    header=(roots['llama']/'vendor/nlohmann/json.hpp').read_text(encoding='utf8').split('#ifndef')[0]
    license_text=(ROOT/'third-party/notices/spdx-MIT-template.txt').read_text(encoding='utf8').replace('<year> <copyright holders>','2013 - 2025 Niels Lohmann')
    (package/'nlohmann-MIT.txt').write_text(header+'\n'+license_text,encoding='utf8')
    print('Windows resident workers ready',package,flush=True)

if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser();parser.add_argument('--vulkan',action='store_true');args=parser.parse_args();prepare(args.vulkan)
