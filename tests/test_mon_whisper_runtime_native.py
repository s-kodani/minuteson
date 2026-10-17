"""Opt-in actual packaged CLI smoke, using existing model + synthetic silence.

No network, microphone, meeting DB, profile apply or formal quality assessment.
"""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
import wave

SPEC = importlib.util.spec_from_file_location("mon_whisper_runtime_native", Path(__file__).resolve().parents[1] / "tools/mon_whisper_runtime.py")
runtime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


@unittest.skipUnless(sys.platform == "darwin" and os.environ.get("MON_WHISPER_RUNTIME_NATIVE") == "1", "explicit native runtime smoke only")
class NativeRuntimeTests(unittest.TestCase):
    def test_actual_packaged_cli_and_existing_model_run_cpu_and_optional_metal_on_silence(self):
        package = Path(os.environ.get("MON_WHISPER_PACKAGE", str(runtime.REPOSITORY / ".cache/whisper-metal-runtime/package")))
        runtime.verify_package(package)
        model = runtime.private(Path(os.environ["MON_WHISPER_MODEL"]))
        before = (model.stat().st_size, model.stat().st_mtime_ns, runtime.digest(model))
        with tempfile.TemporaryDirectory(prefix="mon-whisper-runtime-smoke-", dir="/private/tmp") as temporary:
            work = Path(temporary)
            binary = work / "whisper-cli"
            with tarfile.open(package / "whisper-runtime.tar", "r:") as source, binary.open("xb") as output:
                member = source.getmember("bin/whisper-cli")
                output.write(source.extractfile(member).read(runtime.MAX_PACKAGE + 1))
            binary.chmod(0o700)
            runtime.verify_binary(binary)
            audio = work / "input.wav"
            with wave.open(str(audio), "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(16000)
                output.writeframes(b"\0\0" * 16000 * 10)
            backends = ["cpu", "metal"] if os.environ.get("MON_WHISPER_RUNTIME_METAL_NATIVE") == "1" else ["cpu"]
            for backend in backends:
                args = [str(binary), "--model", str(model), "--file", str(audio),
                        "--language", "ja", "--threads", "1", "--beam-size", "5",
                        "--output-json", "--output-file", str(work / backend)]
                if backend == "cpu":
                    args.append("--no-gpu")
                _, stderr = runtime.command(args, timeout=60)
                if backend == "metal":
                    self.assertIn(b"whisper_backend_init_gpu: using MTL0 backend", stderr)
                else:
                    self.assertNotIn(b"whisper_backend_init_gpu: using MTL0 backend", stderr)
                result = work / f"{backend}.json"
                self.assertTrue(0 < result.stat().st_size < 65536)
                document = json.loads(result.read_text())
                self.assertIsInstance(document["transcription"], list)
        self.assertEqual((model.stat().st_size, model.stat().st_mtime_ns, runtime.digest(model)), before)


if __name__ == "__main__":
    unittest.main()
