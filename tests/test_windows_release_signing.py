"""Public-release preflight must fail before changing an unsigned artifact."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

@unittest.skipUnless(os.name == "nt", "Windows certificate store and PowerShell")
class WindowsReleaseSigning(unittest.TestCase):
    def test_missing_certificate_never_modifies_or_accepts_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory) / "qa.exe"
            original = b"QA-only fixture: no private key or public signature"
            artifact.write_bytes(original)
            environment = dict(os.environ)
            environment.pop("MON_WINDOWS_CERTIFICATE_THUMBPRINT", None)
            environment.pop("MON_WINDOWS_TIMESTAMP_URL", None)
            result = subprocess.run(
                ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                 str(ROOT / "tools/sign-windows.ps1"), "-Path", str(artifact)],
                env=environment, capture_output=True, timeout=30,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"certificate thumbprint", result.stderr)
            self.assertEqual(artifact.read_bytes(), original)

    def test_public_build_rejects_missing_signer_before_creating_outputs(self):
        environment = dict(os.environ)
        environment.pop("MON_WINDOWS_CERTIFICATE_THUMBPRINT", None)
        environment.pop("MON_WINDOWS_TIMESTAMP_URL", None)
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(ROOT / "app/build-windows.ps1")], env=environment, capture_output=True, timeout=30,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"certificate thumbprint", result.stderr)
        self.assertNotIn(b"Compiling", result.stdout)

if __name__ == "__main__":
    unittest.main()
