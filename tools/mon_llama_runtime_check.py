"""Opt-in native checks for the three reviewed runtime-upgrade representatives.

Uses only fixed catalog bytes and synthetic JSON input; no app data or recording.
The report is JSON evidence, never a model or executable distribution artifact.
"""
import argparse
from contextlib import ExitStack, contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time

from mon_windows_parity_assets import Fetch

ROOT = Path(__file__).resolve().parents[1]
REPRESENTATIVES = ('llama-b11533-qwen25-15b-q4km', 'llama-b11533-qwen3-4b-q4km', 'llama-b11533-qwen35-4b-q4km')


@contextmanager
def measured(metrics, phase):
    started = time.monotonic()
    if metrics is not None:
        metrics['stage'] = phase
    try:
        yield
    finally:
        if metrics is not None:
            phases = metrics.setdefault('phaseSeconds', {})
            phases[phase] = phases.get(phase, 0.0) + time.monotonic() - started


def identity(info):
    # Windows path stat may expose creation time as ctime, while fstat exposes
    # metadata-change time. Compare birth time across APIs and ctime on the fd.
    stamp = info.st_birthtime_ns if os.name == 'nt' else info.st_ctime_ns
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, stamp)


def unchanged(source, path, before, message):
    after = os.fstat(source.fileno())
    if (identity(before) != identity(after) or before.st_ctime_ns != after.st_ctime_ns
            or identity(after) != identity(path.lstat())):
        raise ValueError(message)


def verify(path, size, sha, metrics=None):
    with measured(metrics, 'verification'):
        if path.is_symlink() or not path.is_file() or path.stat().st_size != size:
            raise ValueError('native check input size/type mismatch')
        with path.open('rb') as source:
            before = os.fstat(source.fileno())
            if hashlib.file_digest(source, 'sha256').hexdigest() != sha:
                raise ValueError('native check input SHA mismatch')
            unchanged(source, path, before, 'native check input changed during verification')
        return path


def acquire(fetch, url, size, sha, metrics):
    before = dict(getattr(fetch, 'timings', {}))
    started = time.monotonic()
    if metrics is not None:
        metrics['stage'] = 'acquisition'
    try:
        return fetch.acquire(url, size, sha)
    finally:
        if metrics is not None:
            phases = metrics.setdefault('phaseSeconds', {})
            accounted = 0.0
            for source, phase in (('downloadSeconds', 'download'), ('verificationSeconds', 'verification')):
                seconds = getattr(fetch, 'timings', {}).get(source, 0.0) - before.get(source, 0.0)
                phases[phase] = phases.get(phase, 0.0) + seconds
                accounted += seconds
            phases['acquisitionOverhead'] = phases.get('acquisitionOverhead', 0.0) + max(
                0.0, time.monotonic() - started - accounted)


@contextmanager
def checked_archive(path, size, sha, metrics):
    if path.is_symlink():
        raise ValueError('native archive must not be a link')
    with path.open('rb') as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size != size:
            raise ValueError('native archive size/type mismatch')
        with measured(metrics, 'verification'):
            if hashlib.file_digest(source, 'sha256').hexdigest() != sha:
                raise ValueError('native archive SHA mismatch')
            unchanged(source, path, before, 'native archive changed during verification')
        source.seek(0)
        with measured(metrics, 'extraction'):
            archive = tarfile.open(fileobj=source, mode='r:*')
        with archive:
            yield archive
            with measured(metrics, 'extraction'):
                unchanged(source, path, before, 'native archive changed during extraction')


def prepare(choice, destination, fetch, models, metrics=None):
    recipe = choice['recipe']; executable = model = None
    if len(recipe['manifest']['files']) != len(recipe['sources']):
        raise ValueError('native source inventory differs from its manifest')
    # Trust is scoped to this representative only. Keep the checked descriptor
    # open rather than marking a pathname as verified and opening it again.
    archives = {}
    outputs = set()
    with ExitStack() as stack:
        for file, source in zip(recipe['manifest']['files'], recipe['sources']):
            if file['role'] == 'llm':
                candidate = models / (choice['id'].removeprefix('windows-')+'.gguf') if models else None
                model = verify(candidate, file['expectedSize'], file['expectedHash'], metrics) if candidate and candidate.exists() else acquire(fetch, source['url'], file['expectedSize'], file['expectedHash'], metrics)
            elif file['role'] in ('executable', 'runtime_library'):
                name = file['relativePath']
                relative = Path(name)
                if (not name or '\\' in name or ':' in name or relative.is_absolute()
                        or any(part in ('', '.', '..') for part in name.split('/'))):
                    raise ValueError('unsafe native member destination')
                if name.casefold() in outputs:
                    raise ValueError('duplicate native member destination')
                outputs.add(name.casefold())
                archive = source['archive']
                key = (source['url'], archive['expectedSize'], archive['expectedHash'])
                if key not in archives:
                    raw = ROOT/'.cache/windows-runtime/package/runtime.tar' if source['url'].startswith('mon-bundled://windows-runtime-v1/') else acquire(fetch, *key, metrics)
                    archives[key] = stack.enter_context(checked_archive(raw, key[1], key[2], metrics))
                tar = archives[key]
                with measured(metrics, 'extraction'):
                    member = tar.getmember(archive['memberPath'])
                    if not member.isfile() or member.size != file['expectedSize']:
                        raise ValueError('native archive member shape mismatch')
                    output = destination / relative; output.parent.mkdir(parents=True, exist_ok=True)
                    with tar.extractfile(member) as data, output.open('xb') as target:
                        shutil.copyfileobj(data, target)
                verify(output, file['expectedSize'], file['expectedHash'], metrics); output.chmod(0o700)
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
    if (not isinstance(value, dict) or set(value) != {'ok'}
            or type(value['ok']) is not bool or value['ok'] is not expected
            or text[end:].strip() not in ('','[end of text]')):
        raise ValueError('native JSON output did not meet the synthetic schema')


def check(choice, root, fetch, models, worker, metal, metrics=None):
    metrics = {} if metrics is None else metrics
    metrics['stage'] = 'preparation'
    started = time.monotonic()
    try:
        executable,model=prepare(choice,root,fetch,models,metrics)
    finally:
        metrics['preparationSeconds'] = time.monotonic() - started
    family=choice['id'];qwen3='qwen3-' in family or 'qwen35-' in family
    env={k:v for k,v in os.environ.items() if k.upper() in ('SYSTEMROOT','WINDIR','TEMP','TMP','HOME','TMPDIR')}
    if os.name!='nt':env['PATH']='/usr/bin:/bin'
    directory=root/'direct';args=inputs(directory,model,qwen3,metal)
    with measured(metrics, 'direct'):
        with (directory/'result.json').open('wb') as out,(directory/'stderr.log').open('wb') as log:
            command=[str(executable),*args]
            if sys.platform=='darwin':command=['/usr/bin/time','-l',*command]
            subprocess.run(command,cwd=directory,env=env,stdout=out,stderr=log,check=True,timeout=180)
        result(directory/'result.json',True)
        if metal:
            diagnostics=(directory/'stderr.log').read_text(errors='replace')
            if not re.search(r'offloaded [1-9][0-9]*/[1-9][0-9]* layers to GPU',diagnostics):raise ValueError('Metal execution was not observed')
    requests=[dict(protocol=1,id=1,op='init')]
    for index,expected in enumerate((True,False,True),2):
        directory=root/('resident-'+str(index));args=inputs(directory,model,qwen3,metal,expected)
        requests.append(dict(protocol=1,id=index,op='infer',kind='llama',args=args,directory=str(directory),stdout='result.json',stderr='stderr.log'))
    requests.append(dict(protocol=1,id=0,op='shutdown'))
    with measured(metrics, 'resident'):
        completed=subprocess.run([str(worker),'--mon-resident-v1'],input=''.join(json.dumps(r)+'\n' for r in requests).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,timeout=540,check=True)
        replies=[json.loads(row) for row in completed.stdout.splitlines()]
        if any(r['op']=='error' for r in replies) or [r['id'] for r in replies if r['op']=='done'] != [2,3,4]:raise ValueError('resident protocol did not complete every request')
        for index,expected in enumerate((True,False,True),2):result(root/('resident-'+str(index))/'result.json',expected)
        if metal:
            diagnostics='\n'.join((root/('resident-'+str(i))/'stderr.log').read_text(errors='replace') for i in (2,3,4))
            if not re.search(r'offloaded [1-9][0-9]*/[1-9][0-9]* layers to GPU',diagnostics):raise ValueError('resident Metal execution was not observed')
    peak=re.search(r'(\d+)\s+maximum resident set size',(root/'direct/stderr.log').read_text(errors='replace'))
    return dict(choice=family,engineVersion=choice['recipe']['manifest']['engineVersion'],backend='metal' if metal else 'cpu',directSeconds=metrics['phaseSeconds']['direct'],directPeakRssBytes=int(peak[1]) if peak else None,residentThreeRequestsSeconds=metrics['phaseSeconds']['resident'],modelSha256=next(f['expectedHash'] for f in choice['recipe']['manifest']['files'] if f['role']=='llm'))


def write_report(path, records, started, startup_seconds):
    elapsed = time.monotonic() - started
    for record in records:
        record['runTotalSeconds'] = elapsed
        record['startupSeconds'] = startup_seconds
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix(path.suffix + '.pending')
    pending.write_text(json.dumps(records, indent=2) + '\n', encoding='utf8')
    pending.replace(path)


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--models-dir',type=Path)
    parser.add_argument('--metal',action='store_true')
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args(argv)
    report = []
    started = time.monotonic()
    startup_seconds = 0.0
    current = {'choice': None, 'stage': 'startup', 'status': 'running', 'phaseSeconds': {}}
    try:
        catalog=ROOT/'.cache/windows-runtime/package/catalog.json' if os.name=='nt' else ROOT/'app/model-catalog/macos.json'
        worker_root=ROOT/('.cache/windows-runtime/package/workers' if os.name=='nt' else '.cache/inference-runtime/package')
        worker=worker_root/('mon-llama-worker.exe' if os.name=='nt' else 'mon-llama-worker')
        index=json.loads((worker_root/'runtime-index.json').read_text())
        worker_identity=next(r for r in index if r['engine']=='llama')
        verify(worker,worker_identity['bytes'],worker_identity['sha256'],current)
        current['stage'] = 'catalog'
        # Keep model weights outside all Actions cache/artifact upload paths.
        choices=json.loads(catalog.read_text(encoding='utf8'))
        fetch=Fetch(30*1024**3,base=ROOT/'.cache/llama-runtime-check')
        startup_seconds = time.monotonic() - started
        for ident in REPRESENTATIVES:
            current = {'choice': ident, 'stage': 'catalog', 'status': 'running',
                       'phaseSeconds': {}, 'workerSha256': worker_identity['sha256']}
            report.append(current)
            case_started = time.monotonic()
            try:
                choice=next(c for c in choices if c['id'].removeprefix('windows-')==ident)
                current['choice'] = choice['id']
                if choice['recipe']['manifest']['engineVersion']!='b11960':raise ValueError('unreviewed runtime')
                with tempfile.TemporaryDirectory(prefix='mon-llama-runtime-check-') as temporary:
                    try:
                        current.update(check(choice,Path(temporary).resolve(),fetch,args.models_dir.resolve() if args.models_dir else None,worker.resolve(),args.metal,current))
                    except Exception as error:
                        for log in Path(temporary).rglob('stderr.log'):
                            with log.open('rb') as source:
                                source.seek(max(0,log.stat().st_size-4096));print(source.read().decode(errors='replace'),flush=True)
                        if isinstance(error,subprocess.CalledProcessError) and error.stderr:print(error.stderr[-4096:].decode(errors='replace'),flush=True)
                        raise
                current.update(status='passed', stage='complete', exitCode=0)
            finally:
                current['totalSeconds'] = time.monotonic() - case_started
            print(json.dumps(current),flush=True)
            write_report(args.report, report, started, startup_seconds)
        return 0
    except BaseException as error:
        if not report:
            startup_seconds = time.monotonic() - started
            current['totalSeconds'] = startup_seconds
            report.append(current)
        current.update(status='failed', error=f'{type(error).__name__}: {error}',
                       exitCode=130 if isinstance(error, KeyboardInterrupt) else 1)
        raise
    finally:
        write_report(args.report, report, started, startup_seconds)


if __name__=='__main__':sys.exit(main())
