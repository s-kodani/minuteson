"""Exact, component-scoped native intermediates; final packages are not cached."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
RUST_FILES = ('Cargo.toml', 'Cargo.lock', '.cargo/config.toml')
CAPTURE_CRATES = ('crates/mon-contracts', 'crates/mon-core', 'crates/mon-capture-ffi')
COMPONENTS = {
    'windows-runtime': {
        'host': 'win32',
        'files': ('tools/mon_windows_runtime.py', 'tools/mon_windows_workers.py',
                  'tools/mon_windows_vulkan_sdk.py', 'tools/mon_windows_parity_assets.py',
                  'tools/mon_windows_capture_dependencies.py', 'tools/mon_inference_runtime.py',
                  'tools/mon_whisper_runtime.py', 'third-party/notices/spdx-MIT-template.txt'),
        'trees': ('app/model-catalog', 'probes/inference-worker'),
        'paths': ('.cache/windows-runtime/whisper-source', '.cache/windows-runtime/whisper-build',
                  '.cache/windows-runtime/workers-source-*', '.cache/windows-runtime/workers-build-*',
                  '.cache/vkw-*', '.cache/vkl-*'),
    },
    'windows-capture': {
        'host': 'win32',
        'files': (*RUST_FILES, 'tools/mon_windows_capture_dependencies.py'),
        'trees': CAPTURE_CRATES,
        'paths': ('probes/windows-capture/build',),
    },
    'macos-capture': {
        'host': 'darwin',
        'files': (*RUST_FILES, 'probes/macos-capture/Package.swift', 'probes/macos-capture/build-core.sh'),
        'trees': (*CAPTURE_CRATES, 'probes/macos-capture/Sources', 'probes/macos-capture/Tests'),
        'paths': ('probes/macos-capture/.build',),
    },
    'macos-workers': {
        'host': 'darwin',
        'files': ('tools/mon_inference_runtime.py', 'tools/mon_whisper_runtime.py',
                  'third-party/notices/spdx-MIT-template.txt'),
        'trees': ('probes/inference-worker',),
        'paths': ('.cache/inference-runtime/intermediates', '.cache/inference-runtime/downloads'),
    },
}


def input_files(root, component):
    definition = COMPONENTS[component]
    paths = {root / 'tools/mon_ci_cache.py', *(root / name for name in definition['files'])}
    for name in definition['trees']:
        paths.update(p for p in (root / name).rglob('*') if p.is_file())
    if component == 'windows-capture':
        paths.update(p for p in (root / 'probes/windows-capture').glob('*')
                     if p.is_file() and (p.suffix in ('.cpp', '.h') or p.name == 'CMakeLists.txt'))
    return sorted(p for p in paths if p.is_file())


def cache_key(root, identity, component):
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True).encode())
    for path in input_files(root, component):
        digest.update(path.relative_to(root).as_posix().encode() + b'\0')
        digest.update(path.read_bytes() + b'\0')
    return f"native-v2-{component}-{identity['arch']}-{digest.hexdigest()}"


def build_paths(component):
    return list(COMPONENTS[component]['paths'])


def command(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True,
                          encoding='utf8', errors='replace').stdout.strip()


def windows_compilers():
    # CMake's VS generator discovers its toolset even without a developer shell.
    # Cover every installed x64 toolset so an implicit selection cannot change
    # compiler bytes underneath an otherwise identical restored build tree.
    from mon_windows_runtime import visual_studio_installations
    records = []
    for installation in visual_studio_installations():
        for toolset in sorted((installation / 'VC/Tools/MSVC').glob('*')):
            binaries = {}
            for name in ('cl.exe', 'link.exe'):
                path = toolset / 'bin/Hostx64/x64' / name
                if path.is_file():
                    with path.open('rb') as source:
                        binaries[name] = hashlib.file_digest(source, 'sha256').hexdigest()
            if binaries:
                records.append({'path': str(toolset), 'binaries': binaries})
    return records


def toolchain_identity(component):
    if component == 'macos-workers':
        from mon_inference_runtime import build_identity
        cmake = Path(shutil.which('cmake') or '/usr/bin/cmake').resolve()
        return build_identity(cmake, ROOT / '.cache/inference-runtime')
    identity = {'platform': sys.platform, 'arch': platform.machine(),
                'image': os.environ.get('ImageVersion', 'local'),
                'workspace': str(ROOT.resolve()), 'python': platform.python_version(),
                'flags': {key: os.environ.get(key, '') for key in
                          ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS', 'CXXFLAGS',
                           'SDKROOT', 'MACOSX_DEPLOYMENT_TARGET', 'VCToolsVersion', 'WindowsSDKVersion')}}
    if component.endswith('capture'):
        identity['rust'] = command('rustc', '-vV')
    if component == 'macos-capture':
        identity.update(swift=command('xcrun', 'swift', '--version'),
                        clang=command('xcrun', 'clang', '--version'),
                        sdk=command('xcrun', '--show-sdk-version'),
                        developer=os.environ.get('DEVELOPER_DIR', ''))
    else:
        identity.update(cmake=command('cmake', '--version'), compilers=windows_compilers())
    return identity


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--component', choices=COMPONENTS, required=True)
    args = parser.parse_args(argv)
    if COMPONENTS[args.component]['host'] != sys.platform:
        parser.error('the cache component does not match this build host')
    print('key=' + cache_key(ROOT, toolchain_identity(args.component), args.component))
    print('paths<<MON_NATIVE_CACHE_PATHS')
    print('\n'.join(build_paths(args.component)))
    print('MON_NATIVE_CACHE_PATHS')


if __name__ == '__main__':
    main()
