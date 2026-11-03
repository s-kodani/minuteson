"""Exact native-build cache identity; packaged products are never cached here."""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
FILES = (
    'Cargo.toml', 'Cargo.lock', '.cargo/config.toml',
    'tools/mon_ci_cache.py', 'tools/mon_windows_runtime.py',
    'tools/mon_windows_workers.py', 'tools/mon_windows_vulkan_sdk.py',
    'tools/mon_windows_capture_dependencies.py', 'tools/mon_inference_runtime.py',
    'tools/mon_asr_bundle.py', 'tools/mon_model_catalog.py',
    'probes/macos-capture/Package.swift', 'probes/macos-capture/build-core.sh',
)
TREES = ('crates', 'app/model-catalog', 'probes/inference-worker',
         'probes/macos-capture/Sources', 'probes/macos-capture/Tests')


def input_files(root):
    paths = {root / name for name in FILES}
    for name in TREES:
        paths.update(p for p in (root / name).rglob('*') if p.is_file())
    paths.update(p for p in (root / 'probes/windows-capture').glob('*')
                 if p.is_file() and (p.suffix in ('.cpp', '.h') or p.name == 'CMakeLists.txt'))
    return sorted(p for p in paths if p.is_file())


def cache_key(root, identity):
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True).encode())
    for path in input_files(root):
        digest.update(path.relative_to(root).as_posix().encode() + b'\0')
        digest.update(path.read_bytes() + b'\0')
    return f"native-v1-{identity['platform']}-{identity['arch']}-{digest.hexdigest()}"


def build_paths(host):
    if host == 'win32':
        return ['.cache/windows-runtime/whisper-source', '.cache/windows-runtime/whisper-build',
                '.cache/windows-runtime/workers-source-*', '.cache/windows-runtime/workers-build-*',
                '.cache/vkw-*', '.cache/vkl-*', 'probes/windows-capture/build']
    if host == 'darwin':
        return ['probes/macos-capture/.build']
    raise ValueError('native build caching requires Windows or macOS')


def command(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True,
                          encoding='utf8', errors='replace').stdout.strip()


def main():
    identity = {'platform': sys.platform, 'arch': platform.machine(),
                'image': os.environ.get('ImageVersion', 'local'),
                'rust': command('rustc', '-vV'), 'python': platform.python_version(),
                'flags': {key: os.environ.get(key, '') for key in
                          ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS', 'CXXFLAGS',
                           'SDKROOT', 'MACOSX_DEPLOYMENT_TARGET')}}
    if sys.platform == 'darwin':
        identity.update(swift=command('xcrun', 'swift', '--version'),
                        sdk=command('xcrun', '--show-sdk-version'),
                        developer=os.environ.get('DEVELOPER_DIR', ''))
    else:
        identity['cmake'] = command('cmake', '--version')
    print('key=' + cache_key(ROOT, identity))
    print('paths<<MON_NATIVE_CACHE_PATHS')
    print('\n'.join(build_paths(sys.platform)))
    print('MON_NATIVE_CACHE_PATHS')


if __name__ == '__main__':
    main()
