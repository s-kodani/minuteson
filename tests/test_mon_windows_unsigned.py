"""Unsigned release admission, cache isolation and read-only PE verification.

The build tests stop at a mocked npm invocation. PE fixtures are compiled but
never launched; these tests do not create certificates, sign files or alter trust.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "app/build-windows.ps1"
VERIFY = ROOT / "tools/assert-windows-unsigned.ps1"
POWERSHELL = ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]


def powershell(script, *arguments, environment=None):
    return subprocess.run(
        [*POWERSHELL, str(script), *map(str, arguments)], env=environment,
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=45,
    )


BUILD_HARNESS = r'''
param([string]$BuildScript, [string]$CaptureJson, [switch]$Qa, [switch]$Unsigned)
$ErrorActionPreference = 'Stop'
$global:BuildCalls = @()
function Get-Command {
    param([string]$Name, [string]$CommandType, [string]$ErrorAction)
    if ($env:MON_TEST_TOOL_DIRECTORY -and $Name -in @('cmake.exe', 'ctest.exe')) {
        # Hide host-installed tools: discovery must prepend the fixture's VS
        # directory before either executable becomes visible.
        if (($env:PATH -split [IO.Path]::PathSeparator)[0] -eq $env:MON_TEST_TOOL_DIRECTORY) {
            return [pscustomobject]@{ Source = Join-Path $env:MON_TEST_TOOL_DIRECTORY $Name }
        }
        return
    }
    Microsoft.PowerShell.Core\Get-Command @PSBoundParameters
}
function python {
    $global:BuildCalls += [pscustomobject]@{ tool = 'python'; arguments = @($args) }
    $global:LASTEXITCODE = 0
}
function npm {
    $global:BuildCalls += [pscustomobject]@{
        tool = 'npm'; arguments = @($args)
        releaseSigning = $env:MON_WINDOWS_RELEASE_SIGNING
        targetDirectory = $env:CARGO_TARGET_DIR
        inheritedTauriConfig = $env:TAURI_CONFIG
        searchPath = $env:PATH
    }
    $global:LASTEXITCODE = 87
}
$failure = $null
try {
    $arguments = @{}
    if ($Qa) { $arguments.Qa = $true }
    if ($Unsigned) { $arguments.Unsigned = $true }
    & $BuildScript @arguments
} catch { $failure = $_.Exception.Message }
[ordered]@{ failure = $failure; calls = @($global:BuildCalls) } |
    ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $CaptureJson -Encoding UTF8
'''


@unittest.skipUnless(os.name == "nt", "Windows PowerShell build admission")
class UnsignedBuildModeTests(unittest.TestCase):
    def setUp(self):
        (ROOT / "output").mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(prefix="unsigned build test ", dir=ROOT / "output")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        (self.root / "app").mkdir()
        (self.root / "tools").mkdir()
        (self.root / "bin").mkdir()
        shutil.copyfile(BUILD, self.root / "app/build-windows.ps1")
        self.sentinel = self.root / "signer-invoked.txt"
        (self.root / "tools/sign-windows.ps1").write_text(
            "param([switch]$ValidateOnly, [string]$Path)\n"
            "'signer preflight' | Set-Content -LiteralPath $env:MON_TEST_SIGNER_SENTINEL\n"
            "throw 'SIGNER_PREFLIGHT_REQUIRED'\n", encoding="utf-8",
        )
        # Discovery only: npm is intercepted before CTest can ever be launched.
        (self.root / "bin/ctest.exe").write_bytes(b"MZ inert discovery sentinel; never executed")
        (self.root / "bin/cmake.exe").write_bytes(b"MZ inert discovery sentinel; never executed")
        self.harness = self.root / "build-harness.ps1"
        self.harness.write_text(BUILD_HARNESS, encoding="utf-8")

    def invoke(self, *switches, cmake_environment_name=None):
        result_path = self.root / "result.json"
        environment = dict(os.environ)
        environment.pop("MON_TEST_TOOL_DIRECTORY", None)
        environment.update({
            "PATH": str(self.root / "bin") + os.pathsep + environment["PATH"],
            "MON_TEST_SIGNER_SENTINEL": str(self.sentinel),
            "MON_WINDOWS_RELEASE_SIGNING": "0",
            "MON_WINDOWS_CERTIFICATE_THUMBPRINT": "0" * 40,
            "CARGO_TARGET_DIR": str(self.root / "cached-signed-target"),
            "TAURI_CONFIG": '{"bundle":{"windows":{"signCommand":"stale-signer"}}}',
        })
        if cmake_environment_name:
            for name in ("ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"):
                environment[name] = str(self.root / ("empty-" + name))
            installation_root = self.root / cmake_environment_name
            environment[cmake_environment_name] = str(installation_root)
            binary = installation_root / "Microsoft Visual Studio/2022/Enterprise/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin"
            binary.mkdir(parents=True, exist_ok=True)
            for name in ("cmake.exe", "ctest.exe"):
                shutil.copyfile(self.root / "bin" / name, binary / name)
            environment["MON_TEST_TOOL_DIRECTORY"] = str(binary)
        result = powershell(self.harness, "-BuildScript", self.root / "app/build-windows.ps1",
                            "-CaptureJson", result_path, *switches, environment=environment)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result_path.read_text(encoding="utf-8-sig"))

    def test_visual_studio_cmake_discovery_searches_all_program_files_roots(self):
        for name in ("ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"):
            with self.subTest(root=name):
                result = self.invoke("-Unsigned", cmake_environment_name=name)
                self.assertIn("Tauri build failed", result["failure"])
                npm_calls = [call for call in result["calls"] if call["tool"] == "npm"]
                self.assertEqual(len(npm_calls), 1)
                binary = Path(npm_calls[0]["searchPath"].split(os.pathsep)[0])
                self.assertTrue(binary.is_relative_to(self.root / name))
                self.assertTrue((binary / "cmake.exe").is_file())
                self.assertTrue((binary / "ctest.exe").is_file())
                self.assertFalse(self.sentinel.exists())

    def test_qa_and_unsigned_are_rejected_before_signing_or_build_hooks(self):
        result = self.invoke("-Qa", "-Unsigned")
        self.assertIn("cannot be combined", result["failure"])
        self.assertEqual(result["calls"], [])
        self.assertFalse(self.sentinel.exists())
        self.assertFalse((self.root / "output").exists())
        self.assertFalse((self.root / "app/src-tauri/target-unsigned").exists())

    def test_default_release_still_requires_signer_even_with_unsigned_environment(self):
        result = self.invoke()
        self.assertIn("SIGNER_PREFLIGHT_REQUIRED", result["failure"])
        self.assertTrue(self.sentinel.is_file())
        self.assertEqual(result["calls"], [])
        self.assertFalse((self.root / "output").exists())

    def test_explicit_unsigned_bypasses_signer_and_ignores_signed_build_cache(self):
        result = self.invoke("-Unsigned")
        self.assertIn("Tauri build failed", result["failure"])
        self.assertFalse(self.sentinel.exists())
        npm_calls = [call for call in result["calls"] if call["tool"] == "npm"]
        self.assertEqual(len(npm_calls), 1)
        invocation = npm_calls[0]
        self.assertEqual(invocation["releaseSigning"], "0")
        self.assertIsNone(invocation["inheritedTauriConfig"])
        self.assertEqual(Path(invocation["targetDirectory"]).resolve(),
                         self.root / "app/src-tauri/target-unsigned")
        arguments = invocation["arguments"]
        self.assertNotIn("src-tauri/tauri.qa.conf.json", arguments)
        config_arguments = [arguments[index + 1] for index, value in enumerate(arguments) if value == "--config"]
        self.assertEqual(config_arguments[0], "src-tauri/tauri.windows.bundle.conf.json")
        self.assertEqual(len(config_arguments), 2)
        overlay_path = Path(config_arguments[-1])
        self.assertTrue(overlay_path.is_absolute())
        self.assertEqual(overlay_path.parent.resolve(), self.root / "output/windows-unsigned-work")
        self.assertIn(" ", str(overlay_path), "Config-file forwarding must support a workspace containing spaces")
        self.assertTrue(overlay_path.name.endswith("-tauri-config.json"))
        self.assertFalse(overlay_path.read_bytes().startswith(b"\xef\xbb\xbf"), "Tauri config must be UTF-8 without BOM")
        self.assertFalse(any(value.startswith("{") for value in arguments), "Inline JSON is unsafe through Windows npm")
        windows = json.loads(overlay_path.read_text(encoding="utf-8"))["bundle"]["windows"]
        self.assertIn("signCommand", windows)
        self.assertIsNone(windows["signCommand"])
        self.assertIn("certificateThumbprint", windows)
        self.assertIsNone(windows["certificateThumbprint"])
        evidence_calls = [call for call in result["calls"] if call["tool"] == "python"]
        self.assertEqual(len(evidence_calls), 1)
        self.assertIn("windows-unsigned-work", " ".join(evidence_calls[0]["arguments"]))


VERIFY_HARNESS = r'''
param([string]$Verifier, [string]$InputsJson, [string]$Status)
$ErrorActionPreference = 'Stop'
if ($Status) {
    $global:ForcedSignatureStatus = $Status
    function Get-AuthenticodeSignature {
        param([string]$LiteralPath)
        [pscustomobject]@{
            Status = [System.Enum]::Parse([System.Management.Automation.SignatureStatus], $global:ForcedSignatureStatus)
            SignerCertificate = $null; TimeStamperCertificate = $null; Path = $LiteralPath
        }
    }
}
$paths = [string[]](Get-Content -LiteralPath $InputsJson -Raw | ConvertFrom-Json)
@(& $Verifier -Path $paths) | ConvertTo-Json -Depth 5 -Compress
'''


@unittest.skipUnless(os.name == "nt", "Windows PE and Authenticode verification")
class UnsignedPEVerificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        windows = Path(os.environ.get("WINDIR", r"C:\Windows"))
        candidates = [windows / "Microsoft.NET" / name / "v4.0.30319/csc.exe"
                      for name in ("Framework64", "Framework")]
        compiler = next((path for path in candidates if path.is_file()), None)
        if compiler is None:
            raise unittest.SkipTest(".NET Framework C# compiler is needed for an unsigned PE fixture")
        (ROOT / "output").mkdir(exist_ok=True)
        cls.directory = tempfile.TemporaryDirectory(prefix="unsigned-pe-test-", dir=ROOT / "output")
        cls.addClassCleanup(cls.directory.cleanup)
        cls.root = Path(cls.directory.name).resolve()
        source = cls.root / "fixture.cs"
        source.write_text("internal static class Fixture { static void Main() {} }\n", encoding="utf-8")
        binary = cls.root / "unsigned-fixture.exe"
        result = subprocess.run([str(compiler), "/nologo", "/target:exe", "/debug-", "/optimize+",
                                 "/out:" + str(binary), str(source)], capture_output=True, timeout=30)
        if result.returncode:
            raise RuntimeError("Unsigned test fixture compilation failed: " + result.stderr.decode(errors="replace"))
        cls.unsigned = binary.read_bytes()
        cls.harness = cls.root / "verify-harness.ps1"
        cls.harness.write_text(VERIFY_HARNESS, encoding="utf-8")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="case-", dir=self.root)
        self.addCleanup(self.temp.cleanup)
        self.case = Path(self.temp.name)

    def verify(self, *payloads, status=None):
        paths = []
        for index, data in enumerate(payloads):
            path = self.case / f"fixture-{index}.exe"
            path.write_bytes(data)
            paths.append(path)
        inputs = self.case / "inputs.json"
        inputs.write_text(json.dumps([str(path) for path in paths]), encoding="utf-8")
        args = ["-Verifier", VERIFY, "-InputsJson", inputs]
        if status:
            args += ["-Status", status]
        result = powershell(self.harness, *args)
        for path, data in zip(paths, payloads):
            self.assertEqual(path.read_bytes(), data, "Unsigned verifier must never strip or modify files")
        return result

    def test_real_unsigned_pe_files_are_accepted_without_modification(self):
        result = self.verify(self.unsigned, self.unsigned)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        entries = json.loads(result.stdout.lstrip("\ufeff"))
        self.assertEqual(len(entries), 2)
        for entry in entries:
            self.assertEqual(entry["authenticodeStatus"], "NotSigned")
            self.assertFalse(entry["embeddedCertificateTable"])
            self.assertEqual(entry["bytes"], len(self.unsigned))
            self.assertEqual(entry["sha256"], hashlib.sha256(self.unsigned).hexdigest())

    def test_certificate_table_is_rejected_even_if_windows_reports_not_signed(self):
        pe = struct.unpack_from("<I", self.unsigned, 0x3c)[0]
        optional = pe + 24
        magic = struct.unpack_from("<H", self.unsigned, optional)[0]
        security = optional + (96 if magic == 0x10b else 112) + 4 * 8
        for offset, size in ((len(self.unsigned), 40), (len(self.unsigned), 0), (0, 40)):
            with self.subTest(offset=offset, size=size):
                altered = bytearray(self.unsigned)
                struct.pack_into("<II", altered, security, offset, size)
                altered.extend(struct.pack("<IHH", 40, 0x200, 2) + b"certificate table test fixture!!".ljust(32, b"!"))
                result = self.verify(bytes(altered), status="NotSigned")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Embedded certificate table exists", result.stderr)

    def test_every_non_not_signed_status_is_rejected(self):
        for status in ("Valid", "HashMismatch", "NotTrusted", "UnknownError", "NotSupportedFileFormat", "Incompatible"):
            with self.subTest(status=status):
                result = self.verify(self.unsigned, status=status)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Authenticode status must be NotSigned", result.stderr)

    def test_pe_looking_garbage_is_not_accepted_as_unsigned(self):
        for payload in (b"MZnot a PE", b"not an executable", b"MZ" + b"\0" * 510):
            with self.subTest(payload=payload[:20]):
                result = self.verify(payload, status="NotSigned")
                self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
