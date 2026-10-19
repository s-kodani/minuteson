#!/usr/bin/env python3
"""Read-only, local experiment. Never opens the source SQLite database via SQLite.

Reconstruct committed WAL frames in memory, extract ERes2Net features from an
existing mono 16kHz analysis WAV, compare crop/context with clean speaker refs.
Results are evidence for human listening, never production assignments.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import struct
import subprocess
import time
import wave


def snapshot(path):
    wal = Path(str(path) + '-wal')
    for _ in range(3):
        data = path.read_bytes()
        frames = wal.read_bytes() if wal.exists() else b''
        if data == path.read_bytes() and frames == (wal.read_bytes() if wal.exists() else b''):
            break
    else:
        raise RuntimeError('Database changed during capture; retry later')
    output = bytearray(data)
    if frames:
        size = struct.unpack('>I', frames[8:12])[0]
        stride = size + 24
        pending = []
        for offset in range(32, len(frames)-stride+1, stride):
            if frames[offset+8:offset+16] != frames[16:24]:
                break
            page, count = struct.unpack('>II', frames[offset:offset+8])
            pending.append((page, count, frames[offset+24:offset+stride]))
        committed = max((i for i, (_, count, _) in enumerate(pending) if count), default=-1)
        for page, _, payload in pending[:committed+1]:
            if page*size > len(output):
                output.extend(b'\0'*(page*size-len(output)))
            output[(page-1)*size:page*size] = payload
        if committed >= 0:
            output = output[:pending[committed][1]*size]
    output[18:20] = b'\1\1'
    db = sqlite3.connect(':memory:')
    db.deserialize(bytes(output))
    db.row_factory = sqlite3.Row
    assert db.execute('PRAGMA quick_check').fetchone()[0] == 'ok'
    return db


def unit(vector):
    norm = math.sqrt(sum(v*v for v in vector))
    if norm <= 1e-12 or not math.isfinite(norm):
        raise ValueError('Invalid embedding')
    return [v/norm for v in vector]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ['database', 'wave', 'helper', 'library', 'model', 'output']:
        p.add_argument('--'+name, type=Path, required=True)
    for name in ['analysis-id', 'track-id']:
        p.add_argument('--'+name, required=True)
    p.add_argument('--wave-start-us', type=int, default=0)
    a = p.parse_args()
    # Output must be new and must not touch database/audio/model inputs.
    if a.output.exists():
        raise RuntimeError('Output already exists')
    db = snapshot(a.database)
    with wave.open(str(a.wave), 'rb') as wav:
        assert (wav.getframerate(), wav.getnchannels(), wav.getsampwidth()) == (16000, 1, 2)
        end = a.wave_start_us + wav.getnframes()*1_000_000//16000
    segments = [dict(r) for r in db.execute('SELECT speaker_id,start_us,end_us FROM speaker_segments WHERE analysis_revision_id=? AND track_id=? ORDER BY start_us', (a.analysis_id,a.track_id))]
    gaps = list(db.execute('SELECT start_us,end_us FROM audio_gaps WHERE track_id=? AND recovered=0',(a.track_id,)))
    def valid(start,stop):
        return start >= a.wave_start_us and stop <= end and start < stop and not any(g[0]<stop and g[1]>start for g in gaps)
    # Clean reference windows from the whole meeting: longest two solo runs,
    # clipped to 30 seconds; sweep boundaries avoids treating overlap as solo.
    points = sorted({v for s in segments for v in (s['start_us'],s['end_us'])})
    solo = {}
    for start,stop in zip(points,points[1:]):
        ids = {s['speaker_id'] for s in segments if s['start_us']<stop and s['end_us']>start}
        if len(ids)==1 and None not in ids and valid(start,stop):
            key = next(iter(ids))
            rows = solo.setdefault(key,[])
            if rows and rows[-1][1]==start:
                rows[-1]=(rows[-1][0],stop)
            else:
                rows.append((start,stop))
    def extract(start,stop):
        first = (start-a.wave_start_us)*16000//1_000_000
        last = (stop-a.wave_start_us)*16000//1_000_000
        before = time.monotonic()
        result = subprocess.run([str(a.helper),'--protocol-version','1','--library',str(a.library),'--model',str(a.model),'--wave',str(a.wave),'--first-frame',str(first),'--end-frame',str(last),'--threads','2'],capture_output=True,text=True,timeout=120,check=True)
        raw = json.loads(result.stdout)
        assert raw['firstFrame']==first and raw['endFrame']==last
        vector = None if raw['status']!='ready' else unit([struct.unpack('>f',bytes.fromhex(v))[0] for v in raw['vectorBits']])
        return vector, round(time.monotonic()-before,3),raw.get('reason')
    refs = {}
    reference_windows = {}
    for speaker, ranges in solo.items():
        vectors=[]
        reference_windows[speaker]=[]
        for start,stop in sorted(ranges,key=lambda r:r[1]-r[0],reverse=True)[:2]:
            if stop-start < 3_000_000:
                continue
            if stop-start > 30_000_000:
                center=(start+stop)//2;start=center-15_000_000;stop=center+15_000_000
            vector, elapsed, reason = extract(start,stop)
            reference_windows[speaker].append(dict(startUs=start,endUs=stop,seconds=elapsed,unavailable=reason))
            if vector:
                vectors.append(vector)
        if vectors:
            refs[speaker]=unit([sum(v[i] for v in vectors)/len(vectors) for i in range(len(vectors[0]))])
    rows = []
    for u in db.execute("SELECT * FROM utterances WHERE analysis_revision_id=? AND track_id=? AND attribution_state='unknown' ORDER BY start_us",(a.analysis_id,a.track_id)):
        start,stop=u['start_us'],u['end_us']
        if any(s['start_us']<stop and s['end_us']>start for s in segments):
            continue
        # Explicit manual clearing and manually split rows are excluded via
        # retained draft provenance on the current lineage and its ancestors.
        manual=False
        for ancestor in db.execute('WITH RECURSIVE h(id) AS (SELECT ? UNION ALL SELECT parent_revision_id FROM analysis_revisions JOIN h USING(id) WHERE parent_revision_id IS NOT NULL) SELECT id FROM h',(a.analysis_id,)):
            for old in db.execute('SELECT id FROM utterances WHERE analysis_revision_id=? AND lineage_id=?',(ancestor[0],u['lineage_id'])):
                old_ids=sorted(r[0] for r in db.execute('SELECT speaker_id FROM utterance_speakers WHERE utterance_id=?',(old[0],)))
                for d in db.execute("SELECT payload_json FROM working_drafts WHERE base_analysis_id=? AND target_kind='transcript'",(ancestor[0],)):
                    payload=json.loads(d[0])
                    for e in payload.get('utterances',[]):
                        if e['utterance_id']==old[0] and sorted(e['speaker_ids'])!=old_ids:
                            manual=True
                    for split in payload.get('utterance_splits',[]):
                        if any(part['id']==u['lineage_id'] for part in split['parts']):
                            manual=True
        if manual:
            continue
        entry=dict(utteranceId=u['id'],startUs=start,endUs=stop,correctness='unreviewed',variants=[])
        for padding in [0,1_000_000]:
            lo=max(a.wave_start_us,start-padding);hi=min(end,stop+padding)
            if not valid(lo,hi) or hi-lo>30_000_000:
                entry['variants'].append(dict(paddingUs=padding,excluded='gap or range'));continue
            vector,elapsed,reason=extract(lo,hi)
            scores=[] if vector is None else sorted([dict(speakerId=k,similarity=sum(x*y for x,y in zip(vector,v))) for k,v in refs.items() if len(v)==len(vector)],key=lambda r:r['similarity'],reverse=True)
            entry['variants'].append(dict(paddingUs=padding,startUs=lo,endUs=hi,seconds=elapsed,unavailable=reason,scores=scores,margin=(scores[0]['similarity']-scores[1]['similarity']) if len(scores)>1 else None))
        rows.append(entry)
    report=dict(schemaVersion=1,analysisId=a.analysis_id,trackId=a.track_id,modelSha256=hashlib.sha256(a.model.read_bytes()).hexdigest(),references=reference_windows,utterances=rows,applied=False)
    with a.output.open('x') as f:
        json.dump(report,f,ensure_ascii=False,indent=2);f.write('\n')
    print(json.dumps({'utterances':len(rows),'references':len(refs),'output':str(a.output)}))

if __name__=='__main__':
    main()
