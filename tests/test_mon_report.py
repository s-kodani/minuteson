import hashlib
import importlib.util
import io
import json
import shutil
import sys
import tempfile
import unittest
import wave
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
SPEC = importlib.util.spec_from_file_location("mon_report", ROOT / "tools" / "mon_report.py")
mon_report = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(mon_report)
import mon_audio
import mon_eval


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.run = Path(self.temp.name)
        self.config = self.run / "config.toml"
        self.config.write_bytes((ROOT / "evaluation/config.toml").read_bytes())
        self.manifest = {"config_sha256": hashlib.sha256(self.config.read_bytes()).hexdigest(), "measurements": []}
        (self.run / "run.json").write_text(json.dumps(self.manifest), encoding="utf-8")

    def test_no_evidence_is_never_ready(self):
        report = mon_report.build_report(self.config, self.run)
        self.assertEqual(report["recordings"]["expected"], 32)
        self.assertEqual(len(report["recordings"]["missing"]), 32)
        self.assertEqual(report["stability"]["expected"], 8)
        self.assertEqual(len(report["stability"]["missing"]), 8)
        self.assertFalse(report["m1_06_ready_for_decision"])
        self.assertTrue(report["readiness_gaps"])
        self.assertIn("未完了", mon_report.render_markdown(report))

    def test_stability_pass_requires_measurement_and_unchanged_evidence(self):
        config = mon_report.load_config(self.config)
        evidence = self.run / "four-hour-observation.txt"
        evidence.write_text("checked by reviewer", encoding="utf-8")
        result = {"scenario_id": "four-hour-recording", "platform": "macos",
                  "verdict": "pass", "reviewer": "reviewer", "notes": "checked",
                  "evidence_file": evidence.name,
                  "evidence_sha256": hashlib.sha256(evidence.read_bytes()).hexdigest(),
                  "observed_duration_seconds": 14400}
        (self.run / "stability.json").write_text(json.dumps({"results": [result]}), encoding="utf-8")
        self.assertEqual(mon_report.stability_coverage(config, self.run)["reviewed_pass"], 1)
        result["observed_duration_seconds"] = 14390
        (self.run / "stability.json").write_text(json.dumps({"results": [result]}), encoding="utf-8")
        self.assertEqual(mon_report.stability_coverage(config, self.run)["reviewed_pass"], 0)
        result["verdict"] = "fail"
        (self.run / "stability.json").write_text(json.dumps({"results": [result]}), encoding="utf-8")
        self.assertEqual(mon_report.stability_coverage(config, self.run)["reviewed_fail"], 1)
        evidence.write_text("changed", encoding="utf-8")
        self.assertEqual(mon_report.stability_coverage(config, self.run)["reviewed_fail"], 0)

    def test_decision_readiness_requires_both_platforms_profiles_and_scores(self):
        self.manifest["human_review"] = {"status": "completed", "reviewer": "human-1",
                                          "notes": ["quality checked against source"]}
        (self.run / "run.json").write_text(json.dumps(self.manifest), encoding="utf-8")
        config = {"devices": [
            {"id": "mac", "platform": "macos", "status": "available", "required": True, "memory_gib": 16},
            {"id": "win", "platform": "windows", "status": "available", "required": True, "memory_gib": 16}],
            "datasets": [{"id": "data", "status": "available", "required": True, "speaker_count": 10},
                         {"id": "pilot", "status": "available", "required": False, "speaker_count": 3}]}
        coverage = {"expected": 32, "reviewed_pass": 32, "reviewed_fail": 0,
                    "missing": [], "invalid": []}
        stability = {"expected": 8, "reviewed_pass": 8, "reviewed_fail": 0,
                     "missing": [], "invalid": []}
        trials = [
            {"trial_id": f"{device}-{profile}-{run_kind}-{variant}",
             "device_id": device, "dataset_id": "data", "profile": profile,
             "run_kind": run_kind, "variant": variant,
             "model_fingerprint": hashlib.sha256(f"{device}-{profile}".encode()).hexdigest(),
             "status": "timing_met" if profile == "standard" else "measured"}
            for device in ("mac", "win") for profile in ("standard", "high-accuracy")
            for run_kind in ("cold", "warm") for variant in ("baseline", "terminology")
        ]
        benchmarks = {"successful": [], "failed": [], "trials": trials}
        scores = [{"trial_id": trial["trial_id"], "kind": kind, "status": "verified_files"}
                  for trial in trials for kind in ("transcript", "diarization", "minutes")]
        with (patch.object(mon_report, "load_config", return_value=config),
              patch.object(mon_report, "validate_config", return_value=([], [])),
              patch.object(mon_report, "recording_coverage", return_value=coverage),
              patch.object(mon_report, "stability_coverage", return_value=stability),
              patch.object(mon_report, "benchmark_summary", return_value=benchmarks),
              patch.object(mon_report, "quality_summary", return_value=scores)):
            self.assertTrue(mon_report.build_report(self.config, self.run)["m1_06_ready_for_decision"])
            for trial in trials:
                trial["dataset_id"] = "pilot"
            self.assertFalse(mon_report.build_report(self.config, self.run)["m1_06_ready_for_decision"])
            for trial in trials:
                trial["dataset_id"] = "data"
            config["devices"][0]["required"] = False
            self.assertFalse(mon_report.build_report(self.config, self.run)["m1_06_ready_for_decision"])
            config["devices"][0]["required"] = True
            scores.pop()
            self.assertFalse(mon_report.build_report(self.config, self.run)["m1_06_ready_for_decision"])
            scores.append({"trial_id": trials[-1]["trial_id"], "kind": "minutes", "status": "verified_files"})
            trials[-1]["dataset_id"] = "different-data"
            self.assertFalse(mon_report.build_report(self.config, self.run)["m1_06_ready_for_decision"])
            trials[-1]["dataset_id"] = "data"
            trials[-1]["model_fingerprint"] = "f" * 64
            self.assertFalse(mon_report.build_report(self.config, self.run)["m1_06_ready_for_decision"])

    @unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "Audio analysis requires ffmpeg and ffprobe on PATH")
    def test_reviewed_recording_requires_analysis_file(self):
        config = mon_report.load_config(self.config)
        scenario = mon_report.expand_recording_matrix(config)[0]["id"]
        (self.run / "recordings.json").write_text(json.dumps({"results": [{"scenario_id": scenario, "verdict": "pass", "reviewer": "reviewer", "analysis_file": "analysis.json", "notes": "audio reviewed"}]}), encoding="utf-8")
        report = mon_report.build_report(self.config, self.run)
        self.assertEqual(report["recordings"]["reviewed_pass"], 0)
        self.assertEqual(len(report["recordings"]["invalid"]), 1)
        (self.run / "analysis.json").write_text(json.dumps({"schema_version": 1, "microphone_metadata": {}, "system_metadata": {}, "early_window": {}}), encoding="utf-8")
        report = mon_report.build_report(self.config, self.run)
        self.assertEqual(report["recordings"]["reviewed_pass"], 0)
        microphone = self.run / "microphone.caf"
        system = self.run / "system.caf"
        for path in (microphone, system):
            with wave.open(str(path), "wb") as handle:
                handle.setnchannels(1)
                handle.setsampwidth(2)
                handle.setframerate(1000)
                handle.writeframes(b"\0\0" * 900_000)
        analysis = mon_audio.analyze_pair(microphone, system, 0, 6, 500, False)
        analysis["timeline_alignment"] = "file_starts_assumed_equal"
        capture_path = self.run / "manifest.json"
        capture_path.write_text(json.dumps({
            "schemaVersion": 1, "state": "saved", "scenario": scenario,
            "captureMode": "system", "backend": "screencapturekit", "targetPIDs": [],
            "requestedDurationSeconds": 900,
        }), encoding="utf-8")
        analysis["source_manifest"] = str(capture_path.resolve())
        analysis["source_manifest_sha256"] = mon_audio.sha256_file(capture_path)
        (self.run / "analysis.json").write_text(json.dumps(analysis), encoding="utf-8")
        (self.run / "recordings.json").write_text(json.dumps({"results": [{"scenario_id": scenario, "verdict": "pass", "reviewer": "reviewer", "analysis_file": "analysis.json", "notes": "audio reviewed"}]}), encoding="utf-8")
        report = mon_report.build_report(self.config, self.run)
        self.assertEqual(report["recordings"]["reviewed_pass"], 1)
        self.assertEqual(len(report["recordings"]["missing"]), 31)
        original_capture = capture_path.read_bytes()
        wrong_capture = json.loads(original_capture)
        wrong_capture["scenario"] = "macos-meet-edge-system-headset"
        capture_path.write_text(json.dumps(wrong_capture), encoding="utf-8")
        analysis["source_manifest_sha256"] = mon_audio.sha256_file(capture_path)
        (self.run / "analysis.json").write_text(json.dumps(analysis), encoding="utf-8")
        self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_pass"], 0)
        capture_path.write_bytes(original_capture)
        analysis["source_manifest_sha256"] = mon_audio.sha256_file(capture_path)
        (self.run / "analysis.json").write_text(json.dumps(analysis), encoding="utf-8")
        with wave.open(str(microphone), "wb") as handle:
            handle.setnchannels(1)
            handle.setsampwidth(2)
            handle.setframerate(1000)
            handle.writeframes(b"\1\0" * 900_000)
        changed_audio = mon_report.build_report(self.config, self.run)["recordings"]
        self.assertEqual(changed_audio["reviewed_pass"], 0)
        self.assertTrue(any("source audio is missing or changed" in issue for issue in changed_audio["invalid"]))
        with wave.open(str(microphone), "wb") as handle:
            handle.setnchannels(1)
            handle.setsampwidth(2)
            handle.setframerate(1000)
            handle.writeframes(b"\0\0" * 900_000)
        (self.run / "recordings.json").write_text(json.dumps({"results": [{"scenario_id": scenario, "verdict": "fail", "reviewer": "reviewer", "analysis_file": "analysis.json", "notes": "audible echo"}]}), encoding="utf-8")
        self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_fail"], 1)
        (self.run / "recordings.json").write_text(json.dumps({"results": [{"scenario_id": scenario, "verdict": "pass", "reviewer": "reviewer", "analysis_file": "analysis.json", "notes": "audio reviewed"}]}), encoding="utf-8")
        analysis["common_overlap_seconds"] = 10
        (self.run / "analysis.json").write_text(json.dumps(analysis), encoding="utf-8")
        self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_pass"], 0)
        analysis["common_overlap_seconds"] = 950
        (self.run / "analysis.json").write_text(json.dumps(analysis), encoding="utf-8")
        self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_pass"], 0)
        analysis["common_overlap_seconds"] = 900
        (self.run / "analysis.json").write_text(json.dumps(analysis), encoding="utf-8")
        with wave.open(str(system), "wb") as handle:
            handle.setnchannels(1)
            handle.setsampwidth(2)
            handle.setframerate(1000)
            handle.writeframes(b"\0\0" * 1000)
        self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_pass"], 0)

    def test_failed_capture_manifest_can_count_without_audio_analysis(self):
        config = mon_report.load_config(self.config)
        scenarios = mon_report.expand_recording_matrix(config)
        for platform, version_key, reason_key, reason in (
            ("macos", "schemaVersion", "errors", ["permission denied"]),
            ("windows", "schema_version", "error", "device unavailable"),
        ):
            scenario = next(item["id"] for item in scenarios if item["platform"] == platform)
            source = self.run / f"{platform}-failed.json"
            capture = {version_key: 1, "state": "failed", "scenario": scenario,
                       reason_key: reason}
            if platform == "macos":
                capture.update({"captureMode": "system", "requestedDurationSeconds": 900,
                                "backend": "screencapturekit", "targetPIDs": []})
                mode_key, duration_key = "captureMode", "requestedDurationSeconds"
            else:
                capture.update({"mode": "system", "duration_requested_seconds": 900,
                                "process_id": 0})
                mode_key, duration_key = "mode", "duration_requested_seconds"
            source.write_text(json.dumps(capture), encoding="utf-8")
            result = {"scenario_id": scenario, "verdict": "fail", "reviewer": "reviewer",
                      "capture_manifest": source.name,
                      "capture_manifest_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                      "notes": "capture did not start"}
            (self.run / "recordings.json").write_text(json.dumps({"results": [result]}), encoding="utf-8")
            self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_fail"], 1)
            capture[mode_key] = "application"
            source.write_text(json.dumps(capture), encoding="utf-8")
            result["capture_manifest_sha256"] = hashlib.sha256(source.read_bytes()).hexdigest()
            (self.run / "recordings.json").write_text(json.dumps({"results": [result]}), encoding="utf-8")
            self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_fail"], 0)
            capture[mode_key] = "system"
            capture[duration_key] = 30
            source.write_text(json.dumps(capture), encoding="utf-8")
            result["capture_manifest_sha256"] = hashlib.sha256(source.read_bytes()).hexdigest()
            (self.run / "recordings.json").write_text(json.dumps({"results": [result]}), encoding="utf-8")
            self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_fail"], 0)
            capture[duration_key] = 900
            source.write_text(json.dumps({version_key: 1, "state": "saved", "scenario": scenario,
                                          reason_key: reason}), encoding="utf-8")
            self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_fail"], 0)
            result["capture_manifest_sha256"] = hashlib.sha256(source.read_bytes()).hexdigest()
            (self.run / "recordings.json").write_text(json.dumps({"results": [result]}), encoding="utf-8")
            self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_fail"], 0)
            source.write_text(json.dumps({version_key: 1, "state": "failed", "scenario": "other-scenario",
                                          reason_key: reason}), encoding="utf-8")
            result["capture_manifest_sha256"] = hashlib.sha256(source.read_bytes()).hexdigest()
            (self.run / "recordings.json").write_text(json.dumps({"results": [result]}), encoding="utf-8")
            self.assertEqual(mon_report.build_report(self.config, self.run)["recordings"]["reviewed_fail"], 0)

    @unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "Audio analysis requires ffmpeg and ffprobe on PATH")
    def test_windows_audio_analysis_rejects_changed_capture_manifest(self):
        microphone = self.run / "microphone-0000.wav"
        system = self.run / "system-0000.wav"
        for path in (microphone, system):
            with wave.open(str(path), "wb") as handle:
                handle.setnchannels(1)
                handle.setsampwidth(2)
                handle.setframerate(1000)
                handle.writeframes(b"\0\0" * 12000)
        manifest_path = self.run / "manifest.json"
        manifest = {"schema_version": 1, "state": "saved",
                    "scenario": "windows-meet-chrome-system-headset", "mode": "system",
                    "duration_requested_seconds": 900, "process_id": 0,
                    "microphone": {"files": [microphone.name], "first_qpc_100ns": 1000000000,
                                   "timestamp_errors": 0, "discontinuities": 0},
                    "system": {"files": [system.name], "first_qpc_100ns": 1000000000,
                               "timestamp_errors": 0, "discontinuities": 0}}
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        analysis = mon_audio.analyze_pair(microphone, system, 0, 5, 100, False)
        analysis.update({"timeline_alignment": "wasapi_qpc_100ns",
                         "source_manifest": str(manifest_path.resolve()),
                         "source_manifest_sha256": mon_audio.sha256_file(manifest_path)})
        scenario = "windows-meet-chrome-system-headset"
        config = mon_report.load_config(self.config)
        self.assertIsNone(mon_report.validate_audio_analysis(analysis, scenario, config, "fail"))
        manifest["mode"] = "application"
        manifest["process_id"] = 123
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        analysis["source_manifest_sha256"] = mon_audio.sha256_file(manifest_path)
        self.assertIn("scenario or mode does not match",
                      mon_report.validate_audio_analysis(analysis, scenario, config, "fail"))
        manifest["mode"] = "system"
        manifest["process_id"] = 0
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        analysis["source_manifest_sha256"] = mon_audio.sha256_file(manifest_path)
        manifest["note"] = "changed after analysis"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        self.assertIn("manifest is missing or changed",
                      mon_report.validate_audio_analysis(analysis, scenario, config, "fail"))

    @unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "Audio analysis requires ffmpeg and ffprobe on PATH")
    def test_short_failed_recording_can_be_reviewed_without_late_window(self):
        microphone = self.run / "microphone.caf"
        system = self.run / "system.caf"
        for path in (microphone, system):
            with wave.open(str(path), "wb") as handle:
                handle.setnchannels(1)
                handle.setsampwidth(2)
                handle.setframerate(1000)
                handle.writeframes(b"\0\0" * 6000)
        analysis = mon_audio.analyze_pair(microphone, system, 0, 5, 100, False)
        analysis["timeline_alignment"] = "file_starts_assumed_equal"
        scenario = "macos-meet-chrome-system-headset"
        capture_path = self.run / "manifest.json"
        capture_path.write_text(json.dumps({
            "schemaVersion": 1, "state": "saved", "scenario": scenario,
            "captureMode": "system", "backend": "screencapturekit", "targetPIDs": [],
            "requestedDurationSeconds": 900,
        }), encoding="utf-8")
        analysis["source_manifest"] = str(capture_path.resolve())
        analysis["source_manifest_sha256"] = mon_audio.sha256_file(capture_path)
        self.assertIsNone(analysis["late_window"])
        config = mon_report.load_config(self.config)
        self.assertIsNone(mon_report.validate_audio_analysis(analysis, scenario, config, "fail"))
        self.assertIsNotNone(mon_report.validate_audio_analysis(analysis, scenario, config, "pass"))

    def test_run_config_hash_mismatch_is_reported(self):
        self.manifest["config_sha256"] = "wrong"
        (self.run / "run.json").write_text(json.dumps(self.manifest), encoding="utf-8")
        report = mon_report.build_report(self.config, self.run)
        self.assertTrue(any("hash" in error for error in report["configuration_errors"]))

    def test_complete_standard_trial_distinguishes_timing_met_and_missed(self):
        source = self.run / "source.flac"
        source.write_bytes(b"source audio")
        output_paths = {stage: self.run / f"{stage}.out" for stage in mon_report.REQUIRED_STAGES}
        for stage, path in output_paths.items():
            path.write_text(f"{stage} output", encoding="utf-8")
        source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        engine_file = self.run / "engine.bin"
        engine_file.write_bytes(b"fixture executable")
        engine_hash = hashlib.sha256(engine_file.read_bytes()).hexdigest()
        model_file = self.run / "model.bin"
        model_file.write_bytes(b"fixture model")
        model_hash = hashlib.sha256(model_file.read_bytes()).hexdigest()
        settings_file = self.run / "engine-settings.json"
        settings_file.write_text(json.dumps({"schema_version": 1, "engine": "fixture",
                                             "settings": {"threads": 2}}), encoding="utf-8")
        settings_hash = hashlib.sha256(settings_file.read_bytes()).hexdigest()
        config = {
            "thresholds": {"max_standard_processing_ratio": 0.5, "max_minutes_processing_ratio": 1 / 12},
            "devices": [{"id": "mac-16", "status": "available", "required": True, "platform": "macos",
                         "architecture": "arm64", "memory_gib": 16, "cpu": "Fixture CPU", "power_source": "AC"}],
            "datasets": [{"id": "meeting", "status": "available", "required": True,
                          "speaker_count": 10, "consent_confirmed": True,
                          "duration_seconds": 3600, "sha256": source_hash}],
        }
        measurements = []
        primary_inputs = {
            "audio-preparation": source,
            "transcription": output_paths["audio-preparation"],
            "diarization": output_paths["audio-preparation"],
            "integration": output_paths["transcription"],
            "minutes": output_paths["integration"],
        }
        for stage in mon_report.REQUIRED_STAGES:
            primary = primary_inputs[stage]
            output = output_paths[stage]
            measurements.append({
                "stage": stage, "engine": "fixture", "trial_id": "trial-1", "device_id": "mac-16", "dataset_id": "meeting",
                "profile": "standard", "run_kind": "cold", "variant": "baseline",
                "dataset_audio_sha256": source_hash, "input_duration_seconds": 3600,
                "host_facts": {"platform": "macos", "architecture": "arm64", "memory_gib_rounded": 16,
                               "cpu_model": "Fixture CPU", "power_source": "AC", "os_major": 15},
                "host_facts_at_end": {"platform": "macos", "architecture": "arm64", "memory_gib_rounded": 16,
                                      "cpu_model": "Fixture CPU", "power_source": "AC", "os_major": 15},
                "required_host_stable": True,
                "wall_time_seconds": 100, "exit_code": 0, "integrity_ok": True,
                "peak_rss_kib": 1000,
                "model_files": [{"path": str(model_file), "sha256": model_hash,
                                 "changed_during_run": False}],
                "executable_file": {"path": str(engine_file), "sha256": engine_hash,
                                    "changed_during_run": False},
                "engine_settings_file": {"path": str(settings_file), "sha256": settings_hash,
                                         "changed_during_run": False},
                "input_path": str(primary), "input_sha256": hashlib.sha256(primary.read_bytes()).hexdigest(),
                "dependency_files": [{"path": str(output_paths["diarization"]),
                                      "sha256": hashlib.sha256(output_paths["diarization"].read_bytes()).hexdigest(),
                                      "changed_during_run": False}] if stage == "integration" else [],
                "output_files": [{"path": str(output), "sha256": hashlib.sha256(output.read_bytes()).hexdigest()}],
            })
        result = mon_report.evaluate_trial(config, "trial-1", measurements)
        self.assertEqual(result["status"], "timing_met", result["issues"])
        self.assertEqual(result["analysis_seconds"], 400)
        measurements[0]["host_facts"]["power_source"] = "battery"
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        measurements[0]["host_facts"]["power_source"] = "AC"
        measurements[0]["host_facts_at_end"]["power_source"] = "battery"
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        measurements[0]["host_facts_at_end"]["power_source"] = "AC"
        measurements[0]["host_facts"]["os_major"] = 14
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        measurements[0]["host_facts"]["os_major"] = 15
        measurements[0]["host_facts_at_end"]["os_major"] = 16
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        measurements[0]["host_facts_at_end"]["os_major"] = 15
        measurements[-1]["wall_time_seconds"] = 400
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "timing_missed")
        measurements[-1]["wall_time_seconds"] = 100
        measurements[3]["dependency_files"] = []
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        measurements[3]["dependency_files"] = [{"path": str(output_paths["diarization"]),
                                                "sha256": hashlib.sha256(output_paths["diarization"].read_bytes()).hexdigest(),
                                                "changed_during_run": False}]
        output_paths["minutes"].write_text("changed after benchmark", encoding="utf-8")
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        output_paths["minutes"].write_text("minutes output", encoding="utf-8")
        config["devices"][0]["memory_gib"] = 32
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        config["devices"][0]["memory_gib"] = 16
        for item in measurements:
            item["profile"] = "high-accuracy"
            item["wall_time_seconds"] = 1000
        high = mon_report.evaluate_trial(config, "trial-1", measurements)
        self.assertEqual(high["status"], "measured")
        self.assertIsNone(high["analysis_limit_seconds"])
        self.assertIsNone(high["minutes_limit_seconds"])
        alternate_settings = self.run / "alternate-engine-settings.json"
        alternate_settings.write_text(json.dumps({"schema_version": 1, "engine": "fixture",
                                                  "settings": {"threads": 8}}), encoding="utf-8")
        measurements[1]["engine_settings_file"] = {
            "path": str(alternate_settings),
            "sha256": hashlib.sha256(alternate_settings.read_bytes()).hexdigest(),
            "changed_during_run": False,
        }
        different_settings = mon_report.evaluate_trial(config, "trial-1", measurements)
        self.assertEqual(different_settings["status"], "measured")
        self.assertNotEqual(high["model_fingerprint"], different_settings["model_fingerprint"])
        measurements[1]["engine_settings_file"] = {
            "path": str(settings_file), "sha256": settings_hash, "changed_during_run": False,
        }
        model_file.write_bytes(b"different fixture model")
        measurements[1]["model_files"][0]["sha256"] = hashlib.sha256(model_file.read_bytes()).hexdigest()
        changed_model = mon_report.evaluate_trial(config, "trial-1", measurements)
        self.assertEqual(changed_model["status"], "not_comparable")
        model_file.write_bytes(b"fixture model")
        measurements[1]["model_files"][0]["sha256"] = model_hash
        settings_file.write_text(json.dumps({"schema_version": 1, "engine": "fixture",
                                             "settings": {"threads": 8}}), encoding="utf-8")
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        settings_file.write_text(json.dumps({"schema_version": 1, "engine": "fixture",
                                             "settings": {"threads": 2}}), encoding="utf-8")
        engine_file.write_bytes(b"changed executable")
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements)["status"], "not_comparable")
        engine_file.write_bytes(b"fixture executable")
        terms = self.run / "terms.json"
        terms.write_text('{"schema_version":1,"terms":["MON"]}', encoding="utf-8")
        terms_hash = hashlib.sha256(terms.read_bytes()).hexdigest()
        config["datasets"][0]["terms_path"] = str(terms)
        evidence = {"meeting": {"terms_path": {"sha256": terms_hash}}}
        for item in measurements:
            item["variant"] = "terminology"
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements, evidence)["status"], "not_comparable")
        measurements[1]["dependency_files"] = [{"path": str(terms), "sha256": terms_hash,
                                                  "changed_during_run": False}]
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements, evidence)["status"], "measured")
        for item in measurements:
            item["variant"] = "baseline"
        self.assertEqual(mon_report.evaluate_trial(config, "trial-1", measurements, evidence)["status"], "not_comparable")

    def test_benchmark_outputs_form_a_real_five_stage_trial(self):
        source = self.run / "source.bin"
        source.write_bytes(b"fixture audio")
        model = self.run / "model.bin"
        model.write_bytes(b"fixture model")
        source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        self.config.write_text(
            '[[devices]]\nid = "mac-32"\nstatus = "available"\nplatform = "macos"\n'
            'architecture = "arm64"\nmemory_gib = 32\n'
            '[[datasets]]\nid = "fixture"\nstatus = "available"\nrequired = false\nspeaker_count = 3\nconsent_confirmed = true\n'
            f'duration_seconds = 3600\nsha256 = "{source_hash}"\n', encoding="utf-8")
        self.manifest = {
            "config_sha256": hashlib.sha256(self.config.read_bytes()).hexdigest(),
            "dataset_evidence": {"fixture": {"audio_path": {"sha256": source_hash}}},
            "measurements": [],
        }
        (self.run / "run.json").write_text(json.dumps(self.manifest), encoding="utf-8")
        outputs = {stage: self.run / f"{stage}.out" for stage in mon_report.REQUIRED_STAGES}
        inputs = {
            "audio-preparation": source,
            "transcription": outputs["audio-preparation"],
            "diarization": outputs["audio-preparation"],
            "integration": outputs["transcription"],
            "minutes": outputs["integration"],
        }
        facts = {"platform": "macos", "architecture": "arm64", "memory_gib_rounded": 32,
                 "physical_memory_bytes": 32 * 1024 ** 3, "os_version": "test"}
        real_popen = mon_eval.subprocess.Popen
        def spawn(argv, **kwargs):
            if sys.platform != "darwin":
                kwargs["stdin"] = mon_eval.subprocess.PIPE
            return real_popen(argv, **kwargs)
        def sample(process):
            # This tests report provenance, not RSS sampling accuracy. Keep the
            # fixture alive until its first sample, regardless of host load.
            process.stdin.close()
            return 4096
        for stage in mon_report.REQUIRED_STAGES:
            output = outputs[stage]
            script = (f"from pathlib import Path; import sys; "
                      f"Path({str(output)!r}).write_text({stage!r}); "
                      "sys.stdin.buffer.read() if sys.platform != 'darwin' else None")
            args = SimpleNamespace(
                run=str(self.run), stage=stage, engine=f"fixture-{stage}", profile="standard",
                run_kind="cold", variant="baseline", trial_id="candidate",
                device_id="mac-32", dataset_id="fixture",
                model_file=[str(model)] if stage in ("transcription", "diarization", "minutes") else [],
                dependency_file=[str(outputs["diarization"])] if stage == "integration" else [],
                output_file=[str(output)], input=str(inputs[stage]), input_duration=3600,
                cwd=None, executable=["--", sys.executable, "-c", script],
            )
            with (patch.object(mon_eval, "host_facts", return_value=facts),
                  patch.object(mon_eval.subprocess, "Popen", side_effect=spawn),
                  patch.object(mon_eval, "process_rss_kib", side_effect=sample),
                  redirect_stdout(io.StringIO())):
                self.assertEqual(mon_eval.command_benchmark(args), 0)
        recorded = json.loads((self.run / "run.json").read_text(encoding="utf-8"))["measurements"]
        self.assertTrue(all(item["peak_rss_kib"] > 0 for item in recorded))
        result = mon_report.evaluate_trial(mon_report.load_config(self.config), "candidate", recorded, run_dir=self.run)
        self.assertEqual(result["status"], "pilot_measured", result["issues"])
        self.assertTrue(result["pilot"])
        outputs["integration"].write_text("changed after the trial", encoding="utf-8")
        changed = mon_report.evaluate_trial(mon_report.load_config(self.config), "candidate", recorded, run_dir=self.run)
        self.assertEqual(changed["status"], "not_comparable")
        outputs["integration"].write_text("integration", encoding="utf-8")
        log_path = self.run / recorded[0]["stdout_log"]
        log_path.write_text("tampered log", encoding="utf-8")
        changed_log = mon_report.evaluate_trial(mon_report.load_config(self.config), "candidate", recorded, run_dir=self.run)
        self.assertEqual(changed_log["status"], "not_comparable")
        self.assertTrue(any("stdout log" in issue for issue in changed_log["issues"]))
        log_path.write_bytes(b"")
        model.write_bytes(b"tampered model")
        changed_model = mon_report.evaluate_trial(mon_report.load_config(self.config), "candidate", recorded, run_dir=self.run)
        self.assertEqual(changed_model["status"], "not_comparable")
        self.assertTrue(any("model hashes" in issue for issue in changed_model["issues"]))

    def test_missing_stage_does_not_become_a_timing_pass(self):
        result = mon_report.evaluate_trial(
            {"devices": [], "datasets": []}, "partial",
            [{"stage": "transcription", "trial_id": "partial", "wall_time_seconds": 1}],
        )
        self.assertEqual(result["status"], "incomplete")

    def test_attached_quality_score_is_recalculated_and_detects_changes(self):
        reference = self.run / "reference.json"
        hypothesis = self.run / "hypothesis.json"
        terms = self.run / "terms.json"
        reference.write_text(json.dumps({"text": "会議"}), encoding="utf-8")
        hypothesis.write_text(json.dumps({"text": "会議"}), encoding="utf-8")
        terms.write_text(json.dumps({"schema_version": 1, "terms": ["会議"]}), encoding="utf-8")
        reference_hash = hashlib.sha256(reference.read_bytes()).hexdigest()
        hypothesis_hash = hashlib.sha256(hypothesis.read_bytes()).hexdigest()
        terms_hash = hashlib.sha256(terms.read_bytes()).hexdigest()
        self.manifest.update({
            "dataset_evidence": {"meeting": {"transcript_path": {"sha256": reference_hash},
                                             "terms_path": {"sha256": terms_hash}}},
            "measurements": [{"trial_id": "trial-a", "stage": "transcription", "dataset_id": "meeting", "exit_code": 0,
                              "integrity_ok": True, "output_files": [{"path": str(hypothesis),
                                                                        "sha256": hypothesis_hash}]}],
            "scores": [{"kind": "transcript", "trial_id": "trial-a", "dataset_id": "meeting",
                        "reference_path": str(reference), "reference_sha256": reference_hash,
                        "hypothesis_path": str(hypothesis), "hypothesis_sha256": hypothesis_hash,
                        "matches_path": None,
                        "terms_path": str(terms), "terms_sha256": terms_hash,
                        "result": mon_report.score_transcript({"text": "会議"}, {"text": "会議"},
                                                              {"schema_version": 1, "terms": ["会議"]})}],
        })
        (self.run / "run.json").write_text(json.dumps(self.manifest), encoding="utf-8")
        self.assertEqual(mon_report.quality_summary(self.run)[0]["status"], "verified_files")
        self.manifest["scores"][0]["dataset_id"] = "other"
        self.manifest["dataset_evidence"]["other"] = {"transcript_path": {"sha256": reference_hash}}
        (self.run / "run.json").write_text(json.dumps(self.manifest), encoding="utf-8")
        self.assertEqual(mon_report.quality_summary(self.run)[0]["status"], "invalid")
        self.manifest["scores"][0]["dataset_id"] = "meeting"
        (self.run / "run.json").write_text(json.dumps(self.manifest), encoding="utf-8")
        terms.write_text(json.dumps({"schema_version": 1, "terms": ["別の用語"]}), encoding="utf-8")
        self.assertEqual(mon_report.quality_summary(self.run)[0]["status"], "invalid")
        terms.write_text(json.dumps({"schema_version": 1, "terms": ["会議"]}), encoding="utf-8")
        hypothesis.write_text(json.dumps({"text": "変更"}), encoding="utf-8")
        self.assertEqual(mon_report.quality_summary(self.run)[0]["status"], "invalid")


if __name__ == "__main__":
    unittest.main()
