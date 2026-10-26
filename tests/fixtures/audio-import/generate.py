"""Developer-only fixture regeneration; CI consumes checked-in bytes without FFmpeg."""
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent

def box(kind, body):
    return struct.pack('>I4s', len(body) + 8, kind.encode()) + body

def track(identity, kind='soun', refs=None, version=0):
    tkhd = bytes([version, 0, 0, 0]) + bytes(8 if version == 0 else 16) + struct.pack('>I', identity)
    reference = b'' if refs is None else box('tref', box('chap', b''.join(struct.pack('>I', n) for n in refs)))
    return box('trak', box('tkhd', tkhd) + reference + box('mdia', box('hdlr', bytes(8) + kind.encode())))

cases = {
    'sound': (True, box('moov', track(1))),
    'chapter': (True, box('moov', track(1, refs=[2]) + track(2, 'text'))),
    'chapter-v1': (True, box('moov', track(1, refs=[2], version=1) + track(2, 'text', version=1))),
    'two-chapters': (True, box('moov', track(1, refs=[2, 3]) + track(2, 'text') + track(3, 'text'))),
    'unreferenced-text': (False, box('moov', track(1) + track(2, 'text'))),
    'missing-chapter': (False, box('moov', track(1, refs=[2]))),
    'self-reference': (False, box('moov', track(1, refs=[1]) + track(2, 'text'))),
    'duplicate-reference': (False, box('moov', track(1, refs=[2, 2]) + track(2, 'text'))),
    'duplicate-id': (False, box('moov', track(1, refs=[1]) + track(1, 'text'))),
    'zero-id': (False, box('moov', track(0))),
    'unknown-track': (False, box('moov', track(1) + track(2, 'meta'))),
    'video': (False, box('moov', track(1) + track(2, 'vide'))),
    'multi-audio': (False, box('moov', track(1) + track(2))),
    'chapter-references': (False, box('moov', track(1, refs=[2]) + track(2, 'text', [1]))),
    'truncated': (False, box('moov', track(1))[:-1]),
    'invalid-size': (False, struct.pack('>I4s', 7, b'moov')),
    'duplicate-moov': (False, box('moov', track(1)) * 2),
}
entries = []
for name, (accepted, content) in cases.items():
    name = 'structure-' + name + '.m4a'
    (ROOT / name).write_bytes(content)
    entries.append(dict(file=name, accepted=accepted, decode=False))
with tempfile.TemporaryDirectory() as temp:
    meta = Path(temp) / 'chapters.txt'
    meta.write_text(';FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=500\ntitle=First\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=500\nEND=1000\ntitle=Second\n')
    for codec in ('aac', 'alac'):
        name = f'chapters-{codec}.m4a'
        subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i',
                        'sine=frequency=440:sample_rate=48000:duration=1', '-i', str(meta),
                        '-map', '0:a', '-map_metadata', '1', '-map_chapters', '1', '-c:a', codec,
                        '-fflags', '+bitexact', '-flags:a', '+bitexact', str(ROOT / name)], check=True)
        entries.append(dict(file=name, accepted=True, decode=True, codec=codec))
for entry in entries:
    content = (ROOT / entry['file']).read_bytes()
    entry.update(bytes=len(content), sha256=hashlib.sha256(content).hexdigest())
(ROOT / 'manifest.json').write_text(json.dumps(entries, indent=2) + '\n')
