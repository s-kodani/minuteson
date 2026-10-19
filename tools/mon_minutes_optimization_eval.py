#!/usr/bin/env python3
"""Opt-in synthetic content/throughput comparison. Never edits the evidence
allowlist or a real meeting. Content checks are screening; report needs review.
Uses the pinned native worker protocol and current built-in minutes prompts.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import select
import subprocess
import tempfile
import time
import statistics

ROOT = Path(__file__).resolve().parents[1]

class Worker:
    def __init__(self, executable, root, prediction_tokens=8192):
        self.root, self.number = root, 0
        self.prediction_tokens = prediction_tokens
        self.log = (root / 'startup.log').open('w')
        self.child = subprocess.Popen([str(executable), '--mon-resident-v1'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log)
        self.pending = b''
        self.send(dict(protocol=1, id=0, op='init'))
        assert self.receive()['op'] == 'ready'

    def send(self, message):
        self.child.stdin.write((json.dumps(message) + '\n').encode())
        self.child.stdin.flush()

    def receive(self):
        deadline = time.monotonic() + 180
        while b'\n' not in self.pending:
            if time.monotonic() >= deadline or not select.select([self.child.stdout], [], [], min(10, deadline-time.monotonic()))[0]:
                if time.monotonic() >= deadline:
                    raise RuntimeError('worker timeout')
                continue
            block = self.child.stdout.read1(65536)
            if not block:
                raise RuntimeError('worker stopped')
            self.pending += block
            if len(self.pending) > 65536:
                raise RuntimeError('protocol bound exceeded')
        row, self.pending = self.pending.split(b'\n', 1)
        return json.loads(row)

    def run(self, kind, model, system, prompt, backend):
        self.number += 1
        directory = self.root / str(self.number)
        directory.mkdir()
        (directory/'system.txt').write_text(system)
        (directory/'prompt.txt').write_text(prompt)
        (directory/'schema.json').write_text('{"type":"object","properties":{"markdown":{"type":"string"}},"required":["markdown"],"additionalProperties":false}')
        output = 'count.json' if kind == 'llama_tokenize' else 'result.json'
        (directory/output).touch()
        (directory/'stderr.log').touch()
        args = ['--model',str(model),'--file',str(directory/'prompt.txt'),'--system-prompt-file',str(directory/'system.txt'),'--conversation','--jinja','--no-escape','--reasoning','off','--offline']
        if kind != 'llama_tokenize':
            args += ['--json-schema-file',str(directory/'schema.json'),'--threads','4','--threads-batch','4','--ctx-size','32768','--predict',str(self.prediction_tokens),'--gpu-layers','256' if backend == 'metal' else '0','--seed','42','--temp','0','--single-turn','--no-display-prompt','--no-context-shift','--no-warmup','--fit','off','--color','off','--log-colors','off','--simple-io']
            args += ['--device','MTL0','--log-verbosity','4'] if backend == 'metal' else ['--device','none','--no-kv-offload','--no-op-offload']
        self.send(dict(protocol=1,id=self.number,op='infer',kind=kind,args=args,directory=str(directory),stdout=output,stderr='stderr.log'))
        while True:
            response = self.receive()
            if response['id'] != self.number:
                raise RuntimeError('response ID mismatch')
            if response['op'] == 'error':
                raise RuntimeError(str(response) + (directory/'stderr.log').read_text()[-1500:])
            if response['op'] == 'done':
                break
        data = (directory/output).read_text()
        value, end = json.JSONDecoder(strict=False).raw_decode(data.lstrip())
        if data.lstrip()[end:].strip() not in ('', '[end of text]'):
            raise RuntimeError('incomplete/multiple JSON output')
        return value

    def close(self):
        if self.child.poll() is None:
            self.child.stdin.close()
            try:
                self.child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.child.kill()
                self.child.wait(timeout=10)
        self.log.close()
        self.child.stdout.close()


def system_prompt(name):
    text = (ROOT/'app/src-tauri/src/generation/markdown.rs').read_text()
    encoded = re.search(r'const ' + name + r': &str = ("(?:[^"\\]|\\.)*");', text)[1]
    completion=(ROOT/'app/src-tauri/src/generation/markdown_job.rs').read_text()
    suffix=json.loads(re.search(r'const CUSTOM_COMPLETION: &str =\s*("(?:[^"\\]|\\.)*");',completion)[1])
    return json.loads(encoded) + suffix


def fixture(case, facts):
    rows = []
    locations = [0, case['utterances']//2, case['utterances']-1]
    def stamp(n):
        return f'{n//3600:02}:{n//60%60:02}:{n%60:02}'
    for i in range(case['utterances']):
        text = facts[locations.index(i)]['text'] if i in locations else '定例報告の内容を確認しました。追加の決定事項や依頼はありません。'
        start, end = stamp(i*case['stepSeconds']), stamp((i+1)*case['stepSeconds'])
        rows.append(dict(text=text,speakers=['担当者'],audioLink=f'[{start}–{end}](audio:{start}-{end})'))
    return rows


def pipeline(worker, model, rows, extract, merge, backend, optimized):
    groups, current = [], []
    count_calls = 0
    def prompt(rows):
        return json.dumps(dict(meetingTitle='合成の評価会議',startedAtUtcMs=0,utterances=rows),ensure_ascii=False,separators=(',',':'))
    for row in rows:
        candidate = [*current,row]
        text = prompt(candidate)
        if optimized:
            # Count only at a legacy boundary or the final item; exact checks
            # always precede accepting a larger group. Source is never omitted.
            should_count = len(candidate)%100 == 0 or row is rows[-1]
            fits = len((extract+text).encode()) <= 65536
            if fits and should_count:
                fits = worker.run('llama_tokenize',model,extract,text,backend)['tokens'] + 8192 + 512 <= 32768
                count_calls += 1
        else:
            fits = len(candidate) <= 100 and len((extract+text).encode()) + 256 <= 24000
        if not fits:
            if not current:
                raise RuntimeError('fixture does not fit')
            groups.append(current)
            current = [row]
        else:
            current = candidate
    if current:
        groups.append(current)
    kind = 'llama_prefix' if optimized else 'llama'
    drafts = [worker.run(kind,model,extract,prompt(group),backend)['markdown'] for group in groups]
    inference_calls = len(drafts)
    while len(drafts) > 1:
        next_level = []
        for i in range(0,len(drafts),2):
            if i+1 == len(drafts):
                next_level.append(drafts[i])
            else:
                text=json.dumps(dict(drafts=drafts[i:i+2]),ensure_ascii=False,separators=(',',':'))
                next_level.append(worker.run(kind,model,merge,text,backend)['markdown'])
                inference_calls += 1
        drafts=next_level
    return drafts[0], inference_calls, count_calls


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--worker',type=Path,required=True)
    p.add_argument('--model',type=Path,required=True)
    p.add_argument('--backend',choices=['cpu','metal'],default='metal')
    p.add_argument('--runs',type=int,default=3)
    p.add_argument('--prediction-tokens',type=int,default=8192)
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    if not 64 <= args.prediction_tokens <= 8192:
        p.error('prediction-tokens must be 64..8192')
    if not 1 <= args.runs <= 3:
        p.error('runs must be 1..3')
    corpus=json.loads((ROOT/'tests/fixtures/minutes-optimization.json').read_text())
    extract, merge = system_prompt('CONCISE_SYSTEM'), system_prompt('CONCISE_MERGE_SYSTEM')
    report=dict(schemaVersion=1,predictionTokens=args.prediction_tokens,backend=args.backend,runs=args.runs,workerSha256=hashlib.file_digest(args.worker.open('rb'),'sha256').hexdigest(),modelSha256=hashlib.file_digest(args.model.open('rb'),'sha256').hexdigest(),reviewRequired=True,approved=False,cases=[])
    with tempfile.TemporaryDirectory(prefix='mon-minutes-evaluation-') as temp:
        for case in corpus['cases']:
            rows=fixture(case,corpus['facts'])
            allowed={r['audioLink'] for r in rows}
            result=dict(name=case['name'],baseline=[],optimized=[])
            report['cases'].append(result)
            for optimized in [False,True]:
                key='optimized' if optimized else 'baseline'
                for n in range(args.runs):
                    directory=Path(temp)/f'{case["name"]}-{key}-{n}';directory.mkdir()
                    worker=Worker(args.worker.resolve(),directory,args.prediction_tokens)
                    started=time.monotonic()
                    try:
                        text,calls,counts=pipeline(worker,args.model.resolve(),rows,extract,merge,args.backend,optimized)
                        result[key].append(dict(milliseconds=round((time.monotonic()-started)*1000),inferenceCalls=calls,tokenizerCalls=counts,requiredFactsPresent=all(all(t in text for t in f['required']) for f in corpus['facts']),linksValid=bool(re.findall(r'\[[^\]]+\]\(audio:[^)]+\)',text)) and all(link in allowed for link in re.findall(r'\[[^\]]+\]\(audio:[^)]+\)',text)),markdown=text))
                    except (RuntimeError, ValueError, KeyError) as error:
                        result[key].append(dict(milliseconds=round((time.monotonic()-started)*1000),requiredFactsPresent=False,linksValid=False,error=str(error)))
                    finally:
                        worker.close()
                    args.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
                    print(case['name'],key,n+1,result[key][-1]['milliseconds'],flush=True)
            result['medianGainPercent']=100*(1-statistics.median(r['milliseconds'] for r in result['optimized'])/statistics.median(r['milliseconds'] for r in result['baseline']))
    args.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')

if __name__=='__main__':
    main()
