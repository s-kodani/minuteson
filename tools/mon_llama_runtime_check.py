"""Opt-in native checks for the three reviewed runtime-upgrade representatives.

Uses only fixed catalog bytes and synthetic JSON input; no app data or recording.
The report is JSON evidence, never a model or executable distribution artifact.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time

from mon_windows_parity_assets import Fetch

ROOT = Path(__file__).resolve().parents[1]
REPRESENTATIVES = ('llama-b11533-qwen25-15b-q4km', 'llama-b11533-qwen3-4b-q4km', 'llama-b11533-qwen35-4b-q4km')


def verify(path, size, sha):
    if path.is_symlink() or not path.is_file() or path.stat().st_size != size:
        raise ValueError('native check input size/type mismatch')
    with path.open('rb') as source:
        if hashlib.file_digest(source, 'sha256').hexdigest() != sha:
            raise ValueError('native check input SHA mismatch')
    return path


def prepare(choice, destination, fetch, models):
    recipe = choice['recipe']; executable = model = None
    for file, source in zip(recipe['manifest']['files'], recipe['sources']):
        if file['role'] == 'llm':
            candidate = models / (choice['id'].removeprefix('windows-')+'.gguf') if models else None
            model = verify(candidate, file['expectedSize'], file['expectedHash']) if candidate and candidate.exists() else fetch.acquire(source['url'], file['expectedSize'], file['expectedHash'])
        elif file['role'] in ('executable', 'runtime_library'):
            relative = Path(file['relativePath'])
            if relative.is_absolute() or '..' in relative.parts:
                raise ValueError('unsafe native member destination')
            archive = source['archive']
            raw = ROOT/'.cache/windows-runtime/package/runtime.tar' if source['url'].startswith('mon-bundled://windows-runtime-v1/') else fetch.acquire(source['url'], archive['expectedSize'], archive['expectedHash'])
            verify(raw, archive['expectedSize'], archive['expectedHash'])
            with tarfile.open(raw) as tar:
                member = tar.getmember(archive['memberPath'])
                if not member.isfile() or member.size != file['expectedSize']:
                    raise ValueError('native archive member shape mismatch')
                output = destination / relative; output.parent.mkdir(parents=True, exist_ok=True)
                with tar.extractfile(member) as data, output.open('wb') as target:
                    shutil.copyfileobj(data, target)
            verify(output, file['expectedSize'], file['expectedHash']); output.chmod(0o700)
            if file['role'] == 'executable': executable = output
    if executable is None or model is None:
        raise ValueError('representative executable or model missing')
    return executable, model


def inputs(directory, model, qwen3, metal=False, expected=True):
    directory.mkdir()
    (directory/'system.txt').write_text('Return exactly the JSON object requested by the user.', encoding='utf8')
    (directory/'prompt.txt').write_text('Return '+json.dumps({'ok':expected}), encoding='utf8')
    (directory/'schema.json').write_text(json.dumps({'type':'object','properties':{'ok':{'type':'boolean','const':expected}},'required':['ok'],'additionalProperties':False}), encoding='utf8')
    for name in ('result.json','stderr.log'): (directory/name).touch()
    args=['--model',str(model),'--file',str(directory/'prompt.txt'),'--system-prompt-file',str(directory/'system.txt'),'--json-schema-file',str(directory/'schema.json'),'--threads','2','--threads-batch','2','--ctx-size','2048','--predict','64','--gpu-layers','16' if metal else '0','--seed','42','--temp','0.7' if qwen3 else '0','--conversation','--jinja','--single-turn','--no-display-prompt','--no-context-shift','--no-escape','--offline','--no-warmup','--fit','off','--reasoning','off','--color','off','--log-colors','off','--simple-io','--device','MTL0' if metal else 'none']
    if not metal: args+=['--no-kv-offload','--no-op-offload']
    else: args+=['--log-verbosity','4']
    if qwen3:
        # Use the exact production template rather than a second implementation.
        source=(ROOT/'app/src-tauri/src/generation/llama.rs').read_text()
        template=re.search(r'const QWEN3_NON_THINKING_TEMPLATE: &str = r#"(.*?)"#;',source).group(1)
        args+=['--top-p','0.8','--top-k','20','--min-p','0','--presence-penalty','1.5','--chat-template',template]
    return args


def result(path, expected):
    text=path.read_text(encoding='utf8').lstrip()
    value,end=json.JSONDecoder().raw_decode(text)
    if value != {'ok':expected} or text[end:].strip() not in ('','[end of text]'):
        raise ValueError('native JSON output did not meet the synthetic schema')


def check(choice, root, fetch, models, worker, metal):
    executable,model=prepare(choice,root,fetch,models)
    family=choice['id'];qwen3='qwen3-' in family or 'qwen35-' in family
    env={k:v for k,v in os.environ.items() if k.upper() in ('SYSTEMROOT','WINDIR','TEMP','TMP','HOME','TMPDIR')}
    if os.name!='nt':env['PATH']='/usr/bin:/bin'
    directory=root/'direct';args=inputs(directory,model,qwen3,metal)
    started=time.monotonic()
    with (directory/'result.json').open('wb') as out,(directory/'stderr.log').open('wb') as log:
        command=[str(executable),*args]
        if sys.platform=='darwin':command=['/usr/bin/time','-l',*command]
        subprocess.run(command,cwd=directory,env=env,stdout=out,stderr=log,check=True,timeout=180)
    result(directory/'result.json',True);direct=time.monotonic()-started
    if metal:
        diagnostics=(directory/'stderr.log').read_text(errors='replace')
        if not re.search(r'offloaded [1-9][0-9]*/[1-9][0-9]* layers to GPU',diagnostics):raise ValueError('Metal execution was not observed')
    requests=[dict(protocol=1,id=1,op='init')]
    for index,expected in enumerate((True,False,True),2):
        directory=root/('resident-'+str(index));args=inputs(directory,model,qwen3,metal,expected)
        requests.append(dict(protocol=1,id=index,op='infer',kind='llama',args=args,directory=str(directory),stdout='result.json',stderr='stderr.log'))
    requests.append(dict(protocol=1,id=0,op='shutdown'));started=time.monotonic()
    completed=subprocess.run([str(worker),'--mon-resident-v1'],input=''.join(json.dumps(r)+'\n' for r in requests).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,timeout=540,check=True)
    replies=[json.loads(row) for row in completed.stdout.splitlines()]
    if any(r['op']=='error' for r in replies) or [r['id'] for r in replies if r['op']=='done'] != [2,3,4]:raise ValueError('resident protocol did not complete every request')
    for index,expected in enumerate((True,False,True),2):result(root/('resident-'+str(index))/'result.json',expected)
    if metal:
        diagnostics='\n'.join((root/('resident-'+str(i))/'stderr.log').read_text(errors='replace') for i in (2,3,4))
        if not re.search(r'offloaded [1-9][0-9]*/[1-9][0-9]* layers to GPU',diagnostics):raise ValueError('resident Metal execution was not observed')
    peak=re.search(r'(\d+)\s+maximum resident set size',(root/'direct/stderr.log').read_text(errors='replace'))
    return dict(choice=family,engineVersion=choice['recipe']['manifest']['engineVersion'],backend='metal' if metal else 'cpu',directSeconds=direct,directPeakRssBytes=int(peak[1]) if peak else None,residentThreeRequestsSeconds=time.monotonic()-started,modelSha256=next(f['expectedHash'] for f in choice['recipe']['manifest']['files'] if f['role']=='llm'))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--models-dir',type=Path)
    parser.add_argument('--metal',action='store_true')
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args()
    catalog=ROOT/'.cache/windows-runtime/package/catalog.json' if os.name=='nt' else ROOT/'app/model-catalog/macos.json'
    worker_root=ROOT/('.cache/windows-runtime/package/workers' if os.name=='nt' else '.cache/inference-runtime/package')
    worker=worker_root/('mon-llama-worker.exe' if os.name=='nt' else 'mon-llama-worker')
    index=json.loads((worker_root/'runtime-index.json').read_text())
    identity=next(r for r in index if r['engine']=='llama');verify(worker,identity['bytes'],identity['sha256'])
    # Keep model weights outside all Actions cache/artifact upload paths.
    choices=json.loads(catalog.read_text(encoding='utf8'));fetch=Fetch(30*1024**3,base=ROOT/'.cache/llama-runtime-check');report=[]
    for ident in REPRESENTATIVES:
        choice=next(c for c in choices if c['id'].removeprefix('windows-')==ident)
        if choice['recipe']['manifest']['engineVersion']!='b11960':raise ValueError('unreviewed runtime')
        with tempfile.TemporaryDirectory(prefix='mon-llama-runtime-check-') as temporary:
            try:
                item=check(choice,Path(temporary).resolve(),fetch,args.models_dir.resolve() if args.models_dir else None,worker.resolve(),args.metal)
            except Exception as error:
                for log in Path(temporary).rglob('stderr.log'):
                    with log.open('rb') as source:
                        source.seek(max(0,log.stat().st_size-4096));print(source.read().decode(errors='replace'),flush=True)
                if isinstance(error,subprocess.CalledProcessError) and error.stderr:print(error.stderr[-4096:].decode(errors='replace'),flush=True)
                raise
            item['workerSha256']=identity['sha256']
            report.append(item);print(json.dumps(item),flush=True)
    args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n',encoding='utf8')


if __name__=='__main__':main()
