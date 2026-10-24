"""Acquire the pinned JSON parser used by the Windows capture protocol."""
from pathlib import Path
import hashlib
import urllib.request
ROOT=Path(__file__).resolve().parents[1]
def main():
    directory=ROOT/'.cache/windows-capture-headers';directory.mkdir(parents=True,exist_ok=True)
    for name,size,sha,url in (
        ('json.hpp',953436,'aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63','https://raw.githubusercontent.com/nlohmann/json/v3.12.0/single_include/nlohmann/json.hpp'),
        ('LICENSE.MIT',1076,'46a65cffd1ea955132d95a8dd921640714a8d6b537d2e4e482d31145ae95b603','https://raw.githubusercontent.com/nlohmann/json/v3.12.0/LICENSE.MIT')):
        path=directory/name
        if not path.exists():
            from mon_windows_parity_assets import Fetch
            data=Fetch(30*1024**3).acquire(url,size,sha).read_bytes()
            path.write_bytes(data)
        if path.stat().st_size!=size or hashlib.sha256(path.read_bytes()).hexdigest()!=sha:raise ValueError('capture dependency checksum differs')
if __name__=='__main__':main()
