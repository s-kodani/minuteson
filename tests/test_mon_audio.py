import array
import importlib.util
import json
import random
import subprocess
import sys
import tempfile
import unittest
import wave
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[1] / "tools" / "mon_audio.py"
SPEC = importlib.util.spec_from_file_location("mon_audio", MODULE_PATH)
mon_audio = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
sys.modules[SPEC.name] = mon_audio
SPEC.loader.exec_module(mon_audio)


def write_wav(path: Path, samples: list[int]) -> None:
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(mon_audio.SAMPLE_RATE)
        handle.writeframes(array.array("h", samples).tobytes())


class AudioAnalysisTests(unittest.TestCase):
    def test_silence_has_no_fabricated_decibel_value(self):
        metrics = mon_audio.signal_metrics([0.0] * 100)
        self.assertIsNone(metrics["rms_dbfs"])
        self.assertEqual(metrics["near_zero_fraction"], 1)

    def test_unrelated_tracks_have_no_reported_delay(self):
        first = [random.Random(10 + i).random() for i in range(1000)]
        second = [random.Random(10000 + i).random() for i in range(1000)]
        similarity = mon_audio.best_correlation(first, second, 50)
        self.assertIsNone(similarity["lag_ms"])

    def test_recorded_wav_pair_reports_known_delay_and_drift(self):
        rng = random.Random(42)
        source = [rng.randint(-12000, 12000) for _ in range(12000)]
        delayed = [0] * 150 + source[:-150]
        with tempfile.TemporaryDirectory() as directory:
            system = Path(directory) / "system.wav"
            mic = Path(directory) / "microphone.wav"
            write_wav(system, source)
            write_wav(mic, delayed)
            result = mon_audio.analyze_pair(mic, system, 0, 6, 200, True)
            self.assertAlmostEqual(result["early_window"]["cross_track"]["lag_ms"], 150, delta=1)
            self.assertAlmostEqual(result["estimated_drift_ms"], 0, delta=1)
            self.assertGreater(result["early_window"]["cross_track"]["correlation"], 0.95)

    def test_caf_pair_uses_the_same_analysis_path(self):
        rng = random.Random(43)
        source = [rng.randint(-12000, 12000) for _ in range(7000)]
        delayed = [0] * 120 + source[:-120]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wav_system = root / "system.wav"
            wav_mic = root / "microphone.wav"
            write_wav(wav_system, source)
            write_wav(wav_mic, delayed)
            system = root / "system.caf"
            mic = root / "microphone.caf"
            for wav, caf in ((wav_system, system), (wav_mic, mic)):
                subprocess.run(
                    ["ffmpeg", "-nostdin", "-v", "error", "-i", str(wav),
                     "-c:a", "pcm_f32le", str(caf)], check=True,
                )
            result = mon_audio.analyze_pair(mic, system, 0, 5, 200, True)
            self.assertEqual(result["microphone_metadata"]["codec"], "pcm_f32le")
            self.assertAlmostEqual(result["early_window"]["cross_track"]["lag_ms"], 120, delta=1)
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"schemaVersion": 1, "state": "saved"}), encoding="utf-8")
            report = root / "analysis.json"
            self.assertEqual(mon_audio.main([
                "--macos-manifest", str(manifest), "--window-seconds", "5",
                "--max-lag-ms", "200", "--output", str(report),
            ]), 0)
            cli_result = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(cli_result["source_manifest_sha256"], mon_audio.sha256_file(manifest))
            self.assertEqual(cli_result["microphone_files"], [str(mic.resolve())])

    def test_segmented_tracks_analyze_a_window_across_file_boundaries(self):
        rng = random.Random(84)
        source = [rng.randint(-12000, 12000) for _ in range(12000)]
        delayed = [0] * 150 + source[:-150]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            mic_paths = [root / "microphone-0000.wav", root / "microphone-0001.wav"]
            system_paths = [root / "system-0000.wav", root / "system-0001.wav"]
            for paths, samples in ((mic_paths, delayed), (system_paths, source)):
                write_wav(paths[0], samples[:7000])
                write_wav(paths[1], samples[7000:])
            result = mon_audio.analyze_pair(mic_paths, system_paths, 0, 6, 200, True)
            self.assertEqual(len(result["microphone_files"]), 2)
            self.assertAlmostEqual(result["early_window"]["cross_track"]["lag_ms"], 150, delta=1)
            self.assertAlmostEqual(result["late_window"]["cross_track"]["lag_ms"], 150, delta=1)
            self.assertAlmostEqual(result["estimated_drift_ms"], 0, delta=1)
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({
                "schema_version": 1, "state": "saved",
                "microphone": {"files": [path.name for path in mic_paths]},
                "system": {"files": [path.name for path in system_paths]},
            }), encoding="utf-8")
            self.assertEqual(mon_audio.paths_from_windows_manifest(manifest),
                             ([path.resolve() for path in mic_paths], [path.resolve() for path in system_paths]))
            manifest.write_text(json.dumps({
                "schema_version": 1, "state": "saved",
                "microphone": {"files": ["../outside.wav"]},
                "system": {"files": [path.name for path in system_paths]},
            }), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "outside"):
                mon_audio.paths_from_windows_manifest(manifest)

    def test_windows_qpc_offsets_align_tracks_before_correlation(self):
        rng = random.Random(85)
        source = [rng.randint(-12000, 12000) for _ in range(12000)]
        mic_samples = [0] * 50 + source[100:-50]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            mic = root / "microphone-0000.wav"
            system = root / "system-0000.wav"
            write_wav(mic, mic_samples)
            write_wav(system, source)
            manifest = root / "manifest.json"
            document = {
                "schema_version": 1, "state": "saved",
                "microphone": {"files": [mic.name], "first_qpc_100ns": 1_001_000_000,
                               "timestamp_errors": 0, "discontinuities": 0},
                "system": {"files": [system.name], "first_qpc_100ns": 1_000_000_000,
                           "timestamp_errors": 0, "discontinuities": 0},
            }
            manifest.write_text(json.dumps(document), encoding="utf-8")
            offsets = mon_audio.offsets_from_windows_manifest(manifest)
            self.assertEqual(offsets, (0.1, 0.0))
            result = mon_audio.analyze_pair(mic, system, 0, 5, 100, True, *offsets)
            self.assertAlmostEqual(result["early_window"]["cross_track"]["lag_ms"], 50, delta=1)
            self.assertAlmostEqual(result["late_window"]["cross_track"]["lag_ms"], 50, delta=1)
            self.assertAlmostEqual(result["early_window"]["system_file_start_seconds"], 0.1)
            report = root / "analysis.json"
            self.assertEqual(mon_audio.main([
                "--windows-manifest", str(manifest), "--window-seconds", "5",
                "--max-lag-ms", "100", "--calibration-signal-present",
                "--output", str(report),
            ]), 0)
            cli_result = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(cli_result["timeline_alignment"], "wasapi_qpc_100ns")
            self.assertEqual(cli_result["source_manifest_sha256"], mon_audio.sha256_file(manifest))
            self.assertEqual(cli_result["microphone_metadata"]["segments"][0]["sha256"],
                             mon_audio.sha256_file(mic))
            self.assertAlmostEqual(cli_result["early_window"]["cross_track"]["lag_ms"], 50, delta=1)
            document["microphone"]["timestamp_errors"] = 1
            manifest.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "unsafe"):
                mon_audio.offsets_from_windows_manifest(manifest)
            document["microphone"]["timestamp_errors"] = 0
            document["system"]["discontinuities"] = 1
            manifest.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "unsafe"):
                mon_audio.offsets_from_windows_manifest(manifest)

    def test_negative_analysis_window_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "non-negative start"):
            mon_audio.analyze_pair(Path("missing-mic.wav"), Path("missing-system.wav"), -1, 5, 100, False)


if __name__ == "__main__":
    unittest.main()
