"""Fetch the fixed model inputs for Windows parity validation, with a durable
30 GiB additional-network budget (including failed attempts). No app data or
settings are opened. Hash-verified existing model bytes are reused in place.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tarfile
import urllib.request
import zipfile
import time

ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'.cache/windows-parity'
LEDGER=BASE/'download-ledger.json'

def sha(path):
    with path.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def save(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    temporary=path.with_suffix(path.suffix+'.pending')
    temporary.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf8')
    for attempt in range(100):
        try:
            temporary.replace(path)
            return
        except PermissionError:
            if attempt==99:raise
            time.sleep(0.02)

class Fetch:
    def __init__(self,budget,base=None):
        self.limit=budget
        self.base=BASE if base is None else Path(base)
        self.ledger_path=LEDGER if base is None else self.base/'download-ledger.json'
        records=[json.loads(p.read_text(encoding='utf8')) for p in (self.ledger_path,self.ledger_path.with_suffix('.json.pending')) if p.exists()]
        self.ledger=max(records,key=lambda r:r['downloadedBytes']) if records else dict(downloadedBytes=0,attempts=[])
        if self.ledger['downloadedBytes']>budget:raise ValueError('previous network usage exceeds this budget')
    def acquire(self,url,size,digest):
        if not url.startswith('https://'):raise ValueError('fixed HTTPS input required')
        target=self.base/'assets'/digest
        candidates=[target,*list((ROOT/'.cache/windows-runtime/downloads').glob(digest[:16]+'-*'))]
        for p in candidates:
            if p.is_file() and p.stat().st_size==size and sha(p)==digest:return p
        target.parent.mkdir(parents=True,exist_ok=True)
        pending=target.with_suffix('.partial')
        attempt=dict(url=url,expectedBytes=size,receivedBytes=0,state='running')
        self.ledger['attempts'].append(attempt);save(self.ledger_path,self.ledger)
        try:
            request=urllib.request.Request(url,headers={'User-Agent':'MON-Windows-parity-validation'})
            with urllib.request.urlopen(request,timeout=60) as response,pending.open('wb') as output:
                while True:
                    remaining=self.limit-self.ledger['downloadedBytes']
                    if remaining<=0:raise ValueError('additional network budget exhausted')
                    data=response.read(min(1024*1024,remaining))
                    if not data:break
                    attempt['receivedBytes']+=len(data);self.ledger['downloadedBytes']+=len(data)
                    save(self.ledger_path,self.ledger)
                    if attempt['receivedBytes']>size:raise ValueError('fixed input exceeded expected size')
                    output.write(data)
            if pending.stat().st_size!=size or sha(pending)!=digest:raise ValueError('fixed input checksum mismatch')
            pending.replace(target);attempt['state']='verified';return target
        except Exception:
            attempt['state']='failed';raise
        finally:save(self.ledger_path,self.ledger)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--budget-gib',type=int,default=30,choices=range(1,31));parser.add_argument('--all-inputs',action='store_true');args=parser.parse_args()
    choices=json.loads((ROOT/'.cache/windows-runtime/package/catalog.json').read_text(encoding='utf8'))
    wanted={}
    for choice in choices:
        for file,source in zip(choice['recipe']['manifest']['files'],choice['recipe']['sources']):
            if file['role'] in ('asr','vad','llm','segmentation','speaker_embedding') or args.all_inputs and source['url'].startswith('https://'):wanted[file['expectedHash']]=(file,source)
    # Reuse only complete bytes with the exact expected SHA. Filenames are not
    # evidence and partial download directories are not candidates.
    reusable={}
    sizes={f['expectedSize'] for f,_ in wanted.values()}
    for path in (ROOT/'.cache/windows-native-smoke/models').rglob('*'):
        if path.is_file() and path.stat().st_size in sizes:
            digest=sha(path)
            if digest in wanted:reusable[digest]=path
    fetch=Fetch(args.budget_gib*1024**3);index={}
    for digest,(file,source) in wanted.items():
        if args.all_inputs and 'archive' in source:
            archive=source['archive'];raw=fetch.acquire(source['url'],archive['expectedSize'],archive['expectedHash']);index[archive['expectedHash']]=dict(path=str(raw),bytes=archive['expectedSize'],role='archive')
        if digest in reusable:path=reusable[digest]
        elif 'archive' in source:
            archive=source['archive'];raw=fetch.acquire(source['url'],archive['expectedSize'],archive['expectedHash'])
            path=BASE/'assets'/digest
            if not(path.is_file() and path.stat().st_size==file['expectedSize'] and sha(path)==digest):
                if archive['format']=='zip':
                    with zipfile.ZipFile(raw) as z:payload=z.read(archive['memberPath'])
                else:
                    with tarfile.open(raw,'r:*') as tar:
                        member=tar.getmember(archive['memberPath'])
                        if not member.isfile() or member.size!=file['expectedSize']:raise ValueError('model archive member differs')
                        with tar.extractfile(member) as f:payload=f.read(file['expectedSize']+1)
                if len(payload)!=file['expectedSize'] or hashlib.sha256(payload).hexdigest()!=digest:raise ValueError('model member checksum mismatch')
                path.write_bytes(payload)
        else:path=fetch.acquire(source['url'],file['expectedSize'],digest)
        index[digest]=dict(path=str(path),bytes=file['expectedSize'],role=file['role'])
        save(BASE/'asset-index.json',index)
        print(file['relativePath'],file['expectedSize'],'verified; network bytes',fetch.ledger['downloadedBytes'],flush=True)
    print('Verified fixed model assets:',len(index),flush=True)

if __name__=='__main__':main()
