"""Pinned build-only SDK, copied into the workspace without registry/PATH setup.
LunarG's documented copy_only mode supplies the compiler and import library;
GPU drivers and the user's Vulkan loader are never installed or replaced.
"""
import os
from pathlib import Path
import subprocess
from mon_windows_parity_assets import Fetch
ROOT=Path(__file__).resolve().parents[1]
VERSION='1.4.328.1'
SIZE=252243416
SHA='a8675df6d538079c2a719a9373994948091db785b48f142e024254e76348d16c'

def prepare():
    root=ROOT/'.cache/windows-tools'/('VulkanSDK-'+VERSION)
    if (root/'Bin/glslc.exe').is_file() and (root/'Include/vulkan/vulkan.hpp').is_file() and (root/'Lib/vulkan-1.lib').is_file():return root
    installer=Fetch(30*1024**3).acquire(f'https://sdk.lunarg.com/sdk/download/{VERSION}/windows/vulkansdk-windows-X64-{VERSION}.exe',SIZE,SHA)
    # Qt IFW expects an executable extension even for a hash-addressed cache.
    executable=installer.with_suffix('.exe')
    if not executable.exists():
        import shutil
        shutil.copyfile(installer,executable)
    import hashlib
    with executable.open('rb') as stream:
        if hashlib.file_digest(stream,'sha256').hexdigest()!=SHA:raise ValueError('SDK executable differs')
    startup=subprocess.STARTUPINFO();startup.dwFlags|=subprocess.STARTF_USESHOWWINDOW
    subprocess.run([str(executable),'--root',str(root),'--accept-licenses','--default-answer','--confirm-command','install','copy_only=1'],check=True,timeout=600,startupinfo=startup)
    if not(root/'Bin/glslc.exe').is_file() or not(root/'Include/vulkan/vulkan.hpp').is_file() or not(root/'Lib/vulkan-1.lib').is_file():raise ValueError('Vulkan SDK copy is incomplete')
    return root

if __name__=='__main__':print(prepare(),flush=True)
