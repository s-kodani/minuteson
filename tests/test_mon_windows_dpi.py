"""Verify native DPI declarations, including the real linked PE when supplied."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "app/src-tauri/windows.manifest"
NAMESPACES = {
    "asm": "urn:schemas-microsoft-com:asm.v1",
    "asm3": "urn:schemas-microsoft-com:asm.v3",
    "dpi2005": "http://schemas.microsoft.com/SMI/2005/WindowsSettings",
    "dpi2016": "http://schemas.microsoft.com/SMI/2016/WindowsSettings",
}


def windows_sdk_mt():
    sdk = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Windows Kits/10/bin"
    versions = sorted(
        (p for p in sdk.glob("*") if re.fullmatch(r"\d+\.\d+\.\d+\.\d+", p.name)),
        key=lambda p: tuple(map(int, p.name.split("."))), reverse=True,
    )
    for version in versions:
        binary = version / "x64/mt.exe"
        if binary.is_file():
            return binary
    return None


class WindowsDpiManifestTests(unittest.TestCase):
    def assert_dpi_and_native_dialog_contract(self, path):
        root = ET.parse(path).getroot()
        self.assertEqual(root.tag, "{" + NAMESPACES["asm"] + "}assembly")
        settings = root.find("asm3:application/asm3:windowsSettings", NAMESPACES)
        self.assertIsNotNone(settings, "DPI settings must use the native asm.v3 namespace")
        self.assertEqual(settings.findtext("dpi2005:dpiAware", namespaces=NAMESPACES), "true/pm")
        self.assertEqual(settings.findtext("dpi2016:dpiAwareness", namespaces=NAMESPACES), "PerMonitorV2")
        dependency = root.find("asm:dependency/asm:dependentAssembly/asm:assemblyIdentity", NAMESPACES)
        self.assertIsNotNone(dependency, "Native dialogs require Common Controls v6")
        self.assertEqual(dependency.get("name"), "Microsoft.Windows.Common-Controls")
        self.assertEqual(dependency.get("version"), "6.0.0.0")
        self.assertEqual(dependency.get("publicKeyToken"), "6595b64144ccf1df")
        execution = root.find("asm3:trustInfo/asm3:security/asm3:requestedPrivileges/asm3:requestedExecutionLevel", NAMESPACES)
        self.assertIsNotNone(execution)
        self.assertEqual(execution.attrib, {"level": "asInvoker", "uiAccess": "false"})

    def test_declares_per_monitor_v2_and_preserves_standard_user_native_dialogs(self):
        self.assert_dpi_and_native_dialog_contract(MANIFEST)

    @unittest.skipUnless(os.name == "nt", "Windows SDK manifest validation")
    def test_sdk_accepts_the_native_manifest(self):
        mt = windows_sdk_mt()
        if mt is None:
            self.skipTest("Windows SDK x64 mt.exe is unavailable")
        result = subprocess.run([str(mt), "-nologo", "-manifest", str(MANIFEST), "-validate_manifest"],
                                capture_output=True, text=True, errors="replace")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    @unittest.skipUnless(os.name == "nt", "Windows linked executable verification")
    def test_linked_executable_embeds_the_dpi_manifest(self):
        value = os.environ.get("MON_WINDOWS_DPI_BINARY")
        if not value:
            self.skipTest("Set MON_WINDOWS_DPI_BINARY to the freshly built application")
        binary = Path(value).resolve(strict=True)
        mt = windows_sdk_mt()
        self.assertIsNotNone(mt, "Windows SDK x64 mt.exe is required to inspect the built EXE")
        # Keep the temporary output inside the workspace on restricted Windows hosts.
        directory = ROOT / "output"
        directory.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="dpi-manifest-test-", dir=directory) as temporary:
            extracted = Path(temporary) / "embedded.manifest"
            result = subprocess.run([str(mt), "-nologo", f"-inputresource:{binary};#1", f"-out:{extracted}"],
                                    capture_output=True, text=True, errors="replace")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assert_dpi_and_native_dialog_contract(extracted)


if __name__ == "__main__":
    unittest.main()
