#!/usr/bin/env python3
"""Summarize M1 evidence without treating missing or self-reported results as verified."""

import argparse
import hashlib
import json
import math
import re
from pathlib import Path
from typing import Any

from mon_audio import (media_info, offsets_from_windows_manifest, paths_from_macos_manifest,
                       paths_from_windows_manifest)
from mon_eval import (
    expand_recording_matrix, host_meets_required_os, load_config, load_json, score_diarization,
    score_minutes, score_transcript, sha256_if_available, validate_config,
)


def validate_audio_analysis(evidence: dict[str, Any], scenario_id: str, config: dict[str, Any], verdict: str) -> str | None:
    if evidence.get("schema_version") != 1:
        return "invalid audio analysis schema"
    scenario = next((item for item in expand_recording_matrix(config) if item["id"] == scenario_id), None)
    if scenario is None:
        return "unknown recording scenario"
    expected_alignment = "wasapi_qpc_100ns" if scenario_id.startswith("windows-") else "file_starts_assumed_equal"
    if evidence.get("timeline_alignment") != expected_alignment:
        return "audio analysis has no platform-appropriate timeline alignment"
    for name in ("microphone", "system"):
        filenames = evidence.get(f"{name}_files")
        metadata = evidence.get(f"{name}_metadata")
        if not isinstance(filenames, list) or not filenames or not isinstance(metadata, dict):
            return f"{name} audio files or metadata are missing"
        if any(not isinstance(filename, str) or not Path(filename).is_file() for filename in filenames):
            return f"{name} source audio files are missing"
        segments = metadata.get("segments")
        if not isinstance(segments, list) or len(segments) != len(filenames):
            return f"{name} segment metadata is missing"
        actual_duration = 0.0
        for segment, filename in zip(segments, filenames):
            if not isinstance(segment, dict) or segment.get("file") != str(Path(filename).resolve()):
                return f"{name} segment metadata does not match its source file"
            digest = segment.get("sha256")
            if (not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest)
                    or sha256_if_available(Path(filename)) != digest):
                return f"{name} source audio is missing or changed"
            try:
                actual = media_info(Path(filename))
            except (OSError, ValueError, json.JSONDecodeError) as error:
                return f"{name} source audio cannot be read: {error}"
            declared_segment_duration = segment.get("duration_seconds")
            if (isinstance(declared_segment_duration, bool)
                    or not isinstance(declared_segment_duration, (int, float))
                    or not math.isfinite(declared_segment_duration)):
                return f"{name} segment duration is invalid"
            if (actual["codec"] != metadata.get("codec")
                    or actual["sample_rate"] != metadata.get("sample_rate")
                    or actual["channels"] != metadata.get("channels")
                    or abs(actual["duration_seconds"] - declared_segment_duration) > 0.05):
                return f"{name} source audio differs from analysis metadata"
            actual_duration += actual["duration_seconds"]
        duration = metadata.get("duration_seconds")
        if isinstance(duration, bool) or not isinstance(duration, (int, float)) or not math.isfinite(duration) or duration <= 0:
            return f"{name} duration is invalid"
        if abs(actual_duration - duration) > 0.05:
            return f"{name} duration differs from source audio"
    overlap = evidence.get("common_overlap_seconds")
    if isinstance(overlap, bool) or not isinstance(overlap, (int, float)) or not math.isfinite(overlap) or overlap <= 0:
        return "common audio duration is invalid"
    offsets: dict[str, float] = {}
    for name in ("microphone", "system"):
        offset = evidence.get(f"{name}_start_offset_seconds")
        if (isinstance(offset, bool) or not isinstance(offset, (int, float))
                or not math.isfinite(offset) or offset < 0):
            return f"{name} start offset is invalid"
        offsets[name] = float(offset)
    if expected_alignment == "file_starts_assumed_equal" and any(offsets.values()):
        return "macOS direct-track analysis must use equal file starts"
    expected_overlap = min(
        offsets[name] + evidence[f"{name}_metadata"]["duration_seconds"]
        for name in ("microphone", "system")
    ) - max(offsets.values())
    if expected_overlap <= 0 or abs(overlap - expected_overlap) > 0.05:
        return "common audio duration differs from source tracks and start offsets"
    early = evidence.get("early_window")
    if not isinstance(early, dict) or not isinstance(early.get("cross_track"), dict):
        return "early_window is missing"
    if not all(isinstance(early.get(track), dict) for track in ("microphone", "system")):
        return "early_window has no track measurements"
    window_seconds = early.get("duration_seconds")
    if (isinstance(window_seconds, bool) or not isinstance(window_seconds, (int, float))
            or not math.isfinite(window_seconds) or window_seconds <= 0 or window_seconds > overlap):
        return "early_window duration is invalid"
    early_start = early.get("start_seconds")
    common_start = max(offsets.values())
    if (isinstance(early_start, bool) or not isinstance(early_start, (int, float))
            or not math.isfinite(early_start) or early_start < common_start
            or early_start + window_seconds > common_start + overlap + 0.05):
        return "early_window start is invalid"
    late = evidence.get("late_window")
    if late is None and verdict == "fail" and overlap < 2 * window_seconds + early_start - common_start:
        pass
    elif not isinstance(late, dict) or not isinstance(late.get("cross_track"), dict):
        return "late_window is missing"
    elif not all(isinstance(late.get(track), dict) for track in ("microphone", "system")):
        return "late_window has no track measurements"
    if verdict == "pass":
        requested = config.get("recording_matrix", {}).get("duration_seconds")
        tail = config.get("thresholds", {}).get("max_recording_tail_loss_seconds")
        if not isinstance(requested, (int, float)) or not isinstance(tail, (int, float)):
            return "recording duration target is missing"
        if overlap < requested - tail:
            return "recording is shorter than the required duration minus allowed tail loss"
    source = evidence.get("source_manifest")
    source_hash = evidence.get("source_manifest_sha256")
    if not isinstance(source, str) or not Path(source).is_file():
        return "capture manifest is missing"
    if (not isinstance(source_hash, str) or not re.fullmatch(r"[0-9a-f]{64}", source_hash)
            or sha256_if_available(Path(source)) != source_hash):
        return "capture manifest is missing or changed"
    try:
        capture = load_json(Path(source))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        return f"capture manifest is invalid: {error}"
    if expected_alignment == "wasapi_qpc_100ns":
        mode = capture.get("mode")
        requested = capture.get("duration_requested_seconds")
        if capture.get("schema_version") != 1 or capture.get("state") != "saved":
            return "Windows capture manifest must have saved state"
        if scenario_id != capture.get("scenario") or mode != scenario["capture_mode"]:
            return "Windows capture scenario or mode does not match the review"
        process_id = capture.get("process_id")
        if (type(process_id) is not int or
                (mode == "application" and process_id <= 0) or
                (mode == "system" and process_id != 0)):
            return "Windows capture process ID does not match the mode"
        try:
            manifest_paths = paths_from_windows_manifest(Path(source))
            offsets = offsets_from_windows_manifest(Path(source))
        except (OSError, ValueError, json.JSONDecodeError) as error:
            return f"Windows capture manifest is invalid: {error}"
        for name, paths in zip(("microphone", "system"), manifest_paths):
            if [str(path) for path in paths] != evidence[f"{name}_files"]:
                return f"{name} files differ from the Windows capture manifest"
        for name, offset in zip(("microphone", "system"), offsets):
            recorded_offset = evidence.get(f"{name}_start_offset_seconds")
            if (isinstance(recorded_offset, bool) or not isinstance(recorded_offset, (int, float))
                    or not math.isfinite(recorded_offset) or abs(recorded_offset - offset) > 1e-6):
                return f"{name} QPC offset differs from the Windows capture manifest"
    else:
        mode = capture.get("captureMode")
        requested = capture.get("requestedDurationSeconds")
        if capture.get("schemaVersion") != 1 or capture.get("state") != "saved":
            return "macOS capture manifest must have saved state"
        if scenario_id != capture.get("scenario") or mode != scenario["capture_mode"]:
            return "macOS capture scenario or mode does not match the review"
        backend = capture.get("backend")
        pids = capture.get("targetPIDs")
        if backend not in ("screencapturekit", "coreaudio") or not isinstance(pids, list):
            return "macOS capture backend or target PIDs are invalid"
        if ((mode == "system" and (pids or backend != "screencapturekit"))
                or (mode == "application" and (not pids or any(type(pid) is not int or pid <= 0 for pid in pids)))):
            return "macOS capture target PIDs do not match the mode"
        try:
            manifest_paths = paths_from_macos_manifest(Path(source))
        except (OSError, ValueError, json.JSONDecodeError) as error:
            return f"macOS capture manifest is invalid: {error}"
        for name, paths in zip(("microphone", "system"), manifest_paths):
            if [str(path) for path in paths] != evidence[f"{name}_files"]:
                return f"{name} files differ from the macOS capture manifest"
    target_duration = config.get("recording_matrix", {}).get("duration_seconds")
    if (isinstance(requested, bool) or not isinstance(requested, (int, float))
            or not isinstance(target_duration, (int, float))
            or not math.isfinite(requested) or abs(requested - target_duration) > 0.05):
        return "capture requested duration does not match the recording matrix"
    return None


def validate_failed_capture(item: dict[str, Any], scenario_id: str, run_dir: Path,
                            config: dict[str, Any]) -> str | None:
    source = item.get("capture_manifest")
    digest = item.get("capture_manifest_sha256")
    if not isinstance(source, str) or not source or not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        return "failed capture requires a manifest path and SHA-256"
    path = Path(source) if Path(source).is_absolute() else run_dir / source
    if sha256_if_available(path) != digest:
        return "failed capture manifest is missing or changed"
    try:
        manifest = load_json(path)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        return f"failed capture manifest cannot be read: {error}"
    version = manifest.get("schema_version", manifest.get("schemaVersion"))
    if version != 1 or manifest.get("scenario") != scenario_id or manifest.get("state") != "failed":
        return "capture manifest must identify this failed scenario"
    scenario = next((entry for entry in expand_recording_matrix(config) if entry["id"] == scenario_id), None)
    if scenario is None:
        return "failed capture has an unknown scenario"
    if scenario["platform"] == "windows":
        mode = manifest.get("mode")
        requested = manifest.get("duration_requested_seconds")
        process_id = manifest.get("process_id")
        if (type(process_id) is not int or
                (mode == "application" and process_id <= 0) or
                (mode == "system" and process_id != 0)):
            return "failed Windows capture process ID does not match the mode"
    else:
        mode = manifest.get("captureMode")
        requested = manifest.get("requestedDurationSeconds")
        backend = manifest.get("backend")
        pids = manifest.get("targetPIDs")
        if backend not in ("screencapturekit", "coreaudio") or not isinstance(pids, list):
            return "failed macOS capture backend or target PIDs are invalid"
        if ((mode == "system" and (pids or backend != "screencapturekit"))
                or (mode == "application" and (not pids or any(type(pid) is not int or pid <= 0 for pid in pids)))):
            return "failed macOS capture target PIDs do not match the mode"
    target_duration = config.get("recording_matrix", {}).get("duration_seconds")
    if mode != scenario["capture_mode"]:
        return "failed capture mode does not match the review"
    if (isinstance(requested, bool) or not isinstance(requested, (int, float))
            or not isinstance(target_duration, (int, float))
            or not math.isfinite(requested) or abs(requested - target_duration) > 0.05):
        return "failed capture requested duration does not match the recording matrix"
    errors = manifest.get("errors")
    reason = manifest.get("error")
    if not ((isinstance(reason, str) and reason.strip())
            or (isinstance(errors, list) and any(isinstance(error, str) and error.strip() for error in errors))):
        return "failed capture manifest has no error reason"
    return None


def recording_coverage(config: dict[str, Any], run_dir: Path | None) -> dict[str, Any]:
    expected = {item["id"] for item in expand_recording_matrix(config)}
    results_path = run_dir / "recordings.json" if run_dir else None
    results = load_json(results_path).get("results", []) if results_path and results_path.is_file() else []
    if not isinstance(results, list):
        raise ValueError("recordings.json results must be a list")
    reviewed: dict[str, str] = {}
    invalid: list[str] = []
    seen: set[str] = set()
    for index, item in enumerate(results):
        if not isinstance(item, dict):
            invalid.append(f"result {index + 1}: expected an object")
            continue
        scenario_id = item.get("scenario_id")
        if not isinstance(scenario_id, str) or scenario_id not in expected:
            invalid.append(f"result {index + 1}: unknown scenario_id {scenario_id!r}")
            continue
        if scenario_id in seen:
            invalid.append(f"{scenario_id}: duplicate result")
            reviewed.pop(scenario_id, None)
            continue
        seen.add(scenario_id)
        verdict = item.get("verdict")
        reviewer = item.get("reviewer")
        evidence_name = item.get("analysis_file")
        if verdict not in ("pass", "fail") or not isinstance(reviewer, str) or not reviewer.strip():
            invalid.append(f"{scenario_id}: verdict and reviewer are required")
            continue
        if not isinstance(item.get("notes"), str) or not item["notes"].strip():
            invalid.append(f"{scenario_id}: review notes are required")
            continue
        if verdict == "fail" and not evidence_name:
            problem = validate_failed_capture(item, scenario_id, run_dir, config)  # type: ignore[arg-type]
            if problem:
                invalid.append(f"{scenario_id}: {problem}")
            else:
                reviewed[scenario_id] = verdict
            continue
        if not isinstance(evidence_name, str) or not evidence_name:
            invalid.append(f"{scenario_id}: analysis_file is required")
            continue
        evidence_path = (run_dir / evidence_name).resolve()  # type: ignore[operator]
        if not evidence_path.is_relative_to(run_dir.resolve()) or not evidence_path.is_file():  # type: ignore[union-attr]
            invalid.append(f"{scenario_id}: analysis_file must exist inside the run directory")
            continue
        try:
            evidence = load_json(evidence_path)
            problem = validate_audio_analysis(evidence, scenario_id, config, verdict)
        except (OSError, ValueError, json.JSONDecodeError) as error:
            problem = f"audio analysis cannot be read: {error}"
        if problem:
            invalid.append(f"{scenario_id}: {problem}")
            continue
        reviewed[scenario_id] = verdict
    return {
        "expected": len(expected),
        "reviewed_pass": sum(value == "pass" for value in reviewed.values()),
        "reviewed_fail": sum(value == "fail" for value in reviewed.values()),
        "missing": sorted(expected - reviewed.keys()),
        "invalid": invalid,
    }


def stability_coverage(config: dict[str, Any], run_dir: Path | None) -> dict[str, Any]:
    expected = {(scenario["id"], platform)
                for scenario in config.get("stability_scenarios", [])
                for platform in scenario.get("platforms", [])}
    source = run_dir / "stability.json" if run_dir else None
    results = load_json(source).get("results", []) if source and source.is_file() else []
    if not isinstance(results, list):
        raise ValueError("stability.json results must be a list")
    reviewed: dict[tuple[str, str], str] = {}
    invalid: list[str] = []
    seen: set[tuple[str, str]] = set()
    for index, item in enumerate(results):
        if not isinstance(item, dict):
            invalid.append(f"result {index + 1}: expected an object")
            continue
        scenario_id, platform = item.get("scenario_id"), item.get("platform")
        if not isinstance(scenario_id, str) or not isinstance(platform, str):
            invalid.append(f"result {index + 1}: scenario_id and platform must be strings")
            continue
        key = (scenario_id, platform)
        if key not in expected:
            invalid.append(f"result {index + 1}: unknown stability scenario/platform")
            continue
        if key in seen:
            invalid.append(f"{key[0]}/{key[1]}: duplicate result")
            reviewed.pop(key, None)
            continue
        seen.add(key)
        label = f"{key[0]}/{key[1]}"
        if (item.get("verdict") not in ("pass", "fail")
                or not isinstance(item.get("reviewer"), str) or not item["reviewer"].strip()
                or not isinstance(item.get("notes"), str) or not item["notes"].strip()):
            invalid.append(f"{label}: verdict, reviewer, and notes are required")
            continue
        evidence_name = item.get("evidence_file")
        evidence_hash = item.get("evidence_sha256")
        if (not isinstance(evidence_name, str) or not evidence_name
                or not isinstance(evidence_hash, str) or not re.fullmatch(r"[0-9a-f]{64}", evidence_hash)):
            invalid.append(f"{label}: evidence file and SHA-256 are required")
            continue
        evidence_path = Path(evidence_name) if Path(evidence_name).is_absolute() else run_dir / evidence_name  # type: ignore[operator]
        if sha256_if_available(evidence_path) != evidence_hash:
            invalid.append(f"{label}: evidence file is missing or changed")
            continue
        metric = "observed_duration_seconds" if key[0] == "four-hour-recording" else (
            "tail_loss_seconds" if key[0] == "forced-termination-recovery" else None)
        if metric:
            value = item.get(metric)
            if (isinstance(value, bool) or not isinstance(value, (int, float))
                    or not math.isfinite(value) or value < 0):
                invalid.append(f"{label}: {metric} is required")
                continue
            if item["verdict"] == "pass":
                tail = config.get("thresholds", {}).get("max_recording_tail_loss_seconds")
                if not isinstance(tail, (int, float)):
                    invalid.append(f"{label}: stability threshold is missing")
                    continue
                if metric == "observed_duration_seconds":
                    target = next(scenario["duration_seconds"] for scenario in config["stability_scenarios"]
                                  if scenario["id"] == key[0])
                    if not isinstance(target, (int, float)):
                        invalid.append(f"{label}: stability threshold is missing")
                        continue
                    if value < target - tail:
                        invalid.append(f"{label}: observed duration is shorter than the pass threshold")
                        continue
                elif value > tail:
                    invalid.append(f"{label}: tail loss exceeds the pass threshold")
                    continue
        reviewed[key] = item["verdict"]
    return {
        "expected": len(expected),
        "reviewed_pass": sum(value == "pass" for value in reviewed.values()),
        "reviewed_fail": sum(value == "fail" for value in reviewed.values()),
        "missing": [f"{scenario}/{platform}" for scenario, platform in sorted(expected - reviewed.keys())],
        "invalid": invalid,
    }


ANALYSIS_STAGES = ("audio-preparation", "transcription", "diarization", "integration")
REQUIRED_STAGES = (*ANALYSIS_STAGES, "minutes")


def evaluate_trial(
    config: dict[str, Any], trial_id: str, measurements: list[dict[str, Any]],
    dataset_evidence: dict[str, Any] | None = None,
    run_dir: Path | None = None,
) -> dict[str, Any]:
    issues: list[str] = []
    by_stage: dict[str, dict[str, Any]] = {}
    for item in measurements:
        stage = item.get("stage")
        if not isinstance(stage, str) or stage not in REQUIRED_STAGES:
            issues.append(f"unknown stage: {stage}")
        elif stage in by_stage:
            issues.append(f"duplicate stage: {stage}")
        else:
            by_stage[stage] = item
    missing = [stage for stage in REQUIRED_STAGES if stage not in by_stage]
    if missing:
        issues.append("missing stages: " + ", ".join(missing))
    common_fields = ("device_id", "dataset_id", "profile", "run_kind", "variant", "dataset_audio_sha256")
    first = measurements[0]
    for key in common_fields:
        if first.get(key) is None or any(item.get(key) != first.get(key) for item in measurements):
            issues.append(f"inconsistent {key}")
    devices = {item.get("id"): item for item in config.get("devices", [])}
    datasets = {item.get("id"): item for item in config.get("datasets", [])}
    device_id = first.get("device_id")
    dataset_id = first.get("dataset_id")
    device = devices.get(device_id) if isinstance(device_id, str) else None
    dataset = datasets.get(dataset_id) if isinstance(dataset_id, str) else None
    if not device or device.get("status") != "available" or not isinstance(device.get("memory_gib"), int) or device["memory_gib"] <= 0:
        issues.append("trial must use an available device with known physical memory")
    elif device.get("required") and device.get("memory_gib") != 16:
        issues.append("required target trial must use a 16 GiB device")
    for item in measurements:
        host = item.get("host_facts")
        if not isinstance(host, dict) or not device or (
            host.get("platform") != device.get("platform")
            or host.get("architecture") != device.get("architecture")
            or host.get("memory_gib_rounded") != device.get("memory_gib")
        ):
            issues.append(f"{item.get('stage')}: host hardware does not match the configured device")
        elif device.get("required") and (
            not isinstance(host.get("cpu_model"), str)
            or host["cpu_model"].strip().casefold() != str(device.get("cpu", "")).strip().casefold()
            or host.get("power_source") != device.get("power_source")
        ):
            issues.append(f"{item.get('stage')}: CPU model or power source does not match the required device")
        if device and device.get("required") and (not isinstance(host, dict) or not host_meets_required_os(host)):
            issues.append(f"{item.get('stage')}: host OS is below the required minimum or unverified")
        if device and device.get("required"):
            end_host = item.get("host_facts_at_end")
            if (not isinstance(end_host, dict) or not isinstance(host, dict)
                    or item.get("required_host_stable") is not True
                    or any(end_host.get(key) != host.get(key) for key in (
                        "platform", "architecture", "memory_gib_rounded", "cpu_model", "power_source",
                        "os_version", "os_major", "os_build", "os_product_type",
                    ))):
                issues.append(f"{item.get('stage')}: host or power source changed during the stage")
    if not dataset or dataset.get("status") != "available" or not dataset.get("consent_confirmed"):
        issues.append("trial must use an available, consented dataset")
    elif first.get("dataset_audio_sha256") != dataset.get("sha256"):
        issues.append("dataset audio hash does not match configuration")
    duration = dataset.get("duration_seconds") if dataset else None
    if not isinstance(duration, (int, float)) or not math.isfinite(duration) or abs(duration - 3600) > 36:
        issues.append("target trial requires a 60-minute dataset")
    for stage, item in by_stage.items():
        if not isinstance(item.get("engine"), str) or not item["engine"].strip():
            issues.append(f"{stage}: engine is missing")
        executable = item.get("executable_file")
        if (not isinstance(executable, dict) or not isinstance(executable.get("path"), str)
                or not isinstance(executable.get("sha256"), str)
                or not re.fullmatch(r"[0-9a-f]{64}", executable["sha256"])
                or executable.get("changed_during_run") is not False
                or sha256_if_available(Path(executable["path"])) != executable["sha256"]):
            issues.append(f"{stage}: executable is missing or changed")
        settings = item.get("engine_settings_file")
        if (device and device.get("required")) or settings is not None:
            if (not isinstance(settings, dict) or not isinstance(settings.get("path"), str)
                    or not isinstance(settings.get("sha256"), str)
                    or not re.fullmatch(r"[0-9a-f]{64}", settings["sha256"])
                    or settings.get("changed_during_run") is not False
                    or sha256_if_available(Path(settings["path"])) != settings["sha256"]):
                issues.append(f"{stage}: engine settings are missing or changed")
            else:
                try:
                    settings_document = load_json(Path(settings["path"]))
                    if (settings_document.get("schema_version") != 1
                            or settings_document.get("engine") != item.get("engine")
                            or not isinstance(settings_document.get("settings"), dict)
                            or not settings_document["settings"]):
                        issues.append(f"{stage}: engine settings are invalid")
                except (OSError, ValueError, json.JSONDecodeError) as error:
                    issues.append(f"{stage}: engine settings cannot be read: {error}")
        if run_dir:
            for stream in ("stdout", "stderr"):
                name = item.get(f"{stream}_log")
                digest = item.get(f"{stream}_sha256")
                if (not isinstance(name, str) or Path(name).name != name
                        or not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest)
                        or sha256_if_available(run_dir / name) != digest):
                    issues.append(f"{stage}: {stream} log is missing or changed")
        elapsed = item.get("wall_time_seconds")
        if item.get("exit_code") != 0 or item.get("integrity_ok") is not True:
            issues.append(f"{stage}: failed command or input integrity")
        if not isinstance(elapsed, (int, float)) or not math.isfinite(elapsed) or elapsed < 0:
            issues.append(f"{stage}: invalid wall time")
        declared = item.get("input_duration_seconds")
        if not isinstance(declared, (int, float)) or not math.isfinite(declared) or not isinstance(duration, (int, float)) or not math.isfinite(duration) or abs(declared - duration) > max(1.0, duration * 0.01):
            issues.append(f"{stage}: duration mismatch")
        if stage in {"transcription", "diarization", "minutes"}:
            models = item.get("model_files")
            if not isinstance(models, list) or not models or any(
                not isinstance(model, dict) or not isinstance(model.get("path"), str)
                or not isinstance(model.get("sha256"), str)
                or not re.fullmatch(r"[0-9a-f]{64}", model["sha256"])
                or model.get("changed_during_run") is not False
                or sha256_if_available(Path(model["path"])) != model["sha256"]
                for model in models
            ):
                issues.append(f"{stage}: model hashes are missing or changed")
        if not isinstance(item.get("peak_rss_kib"), (int, float)) or item["peak_rss_kib"] <= 0:
            issues.append(f"{stage}: peak RSS is missing")
    inputs_by_stage: dict[str, set[str]] = {}
    dependencies_by_stage: dict[str, set[str]] = {}
    outputs_by_stage: dict[str, set[str]] = {}
    for stage, item in by_stage.items():
        primary_path = item.get("input_path")
        primary_hash = item.get("input_sha256")
        if (not isinstance(primary_path, str) or not isinstance(primary_hash, str)
                or not re.fullmatch(r"[0-9a-f]{64}", primary_hash)
                or sha256_if_available(Path(primary_path)) != primary_hash):
            issues.append(f"{stage}: primary input is missing or changed")
            inputs_by_stage[stage] = set()
        else:
            inputs_by_stage[stage] = {primary_hash}
        dependencies = item.get("dependency_files", [])
        dependencies_by_stage[stage] = set()
        if not isinstance(dependencies, list):
            issues.append(f"{stage}: dependencies are invalid")
            dependencies = []
        for dependency in dependencies:
            if (not isinstance(dependency, dict) or not isinstance(dependency.get("path"), str)
                    or not isinstance(dependency.get("sha256"), str)
                    or sha256_if_available(Path(dependency["path"])) != dependency["sha256"]
                    or dependency.get("changed_during_run") is not False):
                issues.append(f"{stage}: dependency is missing or changed")
            else:
                inputs_by_stage[stage].add(dependency["sha256"])
                dependencies_by_stage[stage].add(dependency["sha256"])
        outputs = item.get("output_files")
        if not isinstance(outputs, list) or not outputs:
            issues.append(f"{stage}: output files are missing")
            outputs_by_stage[stage] = set()
        else:
            outputs_by_stage[stage] = set()
            for output in outputs:
                if (not isinstance(output, dict) or not isinstance(output.get("path"), str)
                        or not isinstance(output.get("sha256"), str)
                        or sha256_if_available(Path(output["path"])) != output["sha256"]):
                    issues.append(f"{stage}: output file is missing or changed")
                else:
                    outputs_by_stage[stage].add(output["sha256"])
    if not missing and dataset:
        if dataset.get("terms_path"):
            terms_evidence = (dataset_evidence or {}).get(dataset_id, {}).get("terms_path")
            if not isinstance(terms_evidence, dict) or not isinstance(terms_evidence.get("sha256"), str):
                issues.append("frozen dataset terminology list is missing")
            else:
                has_terms_dependency = terms_evidence["sha256"] in dependencies_by_stage["transcription"]
                if first.get("variant") == "terminology" and not has_terms_dependency:
                    issues.append("terminology transcription does not declare the frozen terms")
                if first.get("variant") == "baseline" and has_terms_dependency:
                    issues.append("baseline transcription declares the frozen terms")
        if by_stage["audio-preparation"].get("input_sha256") != dataset.get("sha256"):
            issues.append("audio-preparation must consume the frozen dataset audio")
        for stage in ("transcription", "diarization"):
            if by_stage[stage].get("input_sha256") not in outputs_by_stage["audio-preparation"]:
                issues.append(f"{stage} must consume audio-preparation output")
        integration_inputs = inputs_by_stage["integration"]
        for stage in ("transcription", "diarization"):
            if not integration_inputs.intersection(outputs_by_stage[stage]):
                issues.append(f"integration must consume {stage} output")
        if by_stage["minutes"].get("input_sha256") not in outputs_by_stage["integration"]:
            issues.append("minutes must consume integration output")
    def model_hashes(stage: str) -> list[str]:
        files = by_stage[stage].get("model_files")
        if not isinstance(files, list):
            return []
        return sorted(model["sha256"] for model in files
                      if isinstance(model, dict) and isinstance(model.get("sha256"), str))

    model_configuration = {
        stage: {
            "engine": by_stage[stage].get("engine"),
            "executable_sha256": (
                by_stage[stage]["executable_file"].get("sha256")
                if isinstance(by_stage[stage].get("executable_file"), dict) else None
            ),
            "model_sha256": model_hashes(stage),
            "engine_settings_sha256": (
                by_stage[stage]["engine_settings_file"].get("sha256")
                if isinstance(by_stage[stage].get("engine_settings_file"), dict) else None
            ),
        }
        for stage in REQUIRED_STAGES if stage in by_stage
    }
    model_fingerprint = (
        hashlib.sha256(json.dumps(model_configuration, sort_keys=True).encode("utf-8")).hexdigest()
        if not missing and not issues else None
    )
    analysis_seconds = sum(by_stage[stage]["wall_time_seconds"] for stage in ANALYSIS_STAGES if stage in by_stage and isinstance(by_stage[stage].get("wall_time_seconds"), (int, float)))
    minutes_seconds = by_stage["minutes"].get("wall_time_seconds") if "minutes" in by_stage else None
    limits = config.get("thresholds", {})
    analysis_limit = 3600 * limits.get("max_standard_processing_ratio", 0.5)
    minutes_limit = 3600 * limits.get("max_minutes_processing_ratio", 1 / 12)
    profile = first.get("profile")
    if profile not in ("standard", "high-accuracy"):
        issues.append("unknown model profile")
    pilot = not (device and device.get("required") and device.get("memory_gib") == 16
                 and dataset and dataset.get("required") and dataset.get("speaker_count") == 10)
    if issues:
        status = "incomplete" if missing else "not_comparable"
    elif pilot:
        status = "pilot_measured"
    elif profile == "high-accuracy":
        status = "measured"
    elif analysis_seconds <= analysis_limit + 1e-6 and minutes_seconds <= minutes_limit + 1e-6:
        status = "timing_met"
    else:
        status = "timing_missed"
    return {
        "trial_id": trial_id,
        "device_id": first.get("device_id"),
        "dataset_id": first.get("dataset_id"),
        "profile": first.get("profile"),
        "run_kind": first.get("run_kind"),
        "variant": first.get("variant"),
        "model_fingerprint": model_fingerprint,
        "pilot": pilot,
        "status": status,
        "analysis_seconds": analysis_seconds if not missing else None,
        "analysis_limit_seconds": analysis_limit if profile == "standard" else None,
        "minutes_seconds": minutes_seconds,
        "minutes_limit_seconds": minutes_limit if profile == "standard" else None,
        "issues": issues,
    }


def benchmark_summary(config: dict[str, Any], run_dir: Path | None) -> dict[str, Any]:
    path = run_dir / "run.json" if run_dir else None
    manifest = load_json(path) if path and path.is_file() else {}
    measurements = manifest.get("measurements", [])
    if not isinstance(measurements, list):
        raise ValueError("run.json measurements must be a list")
    if any(not isinstance(item, dict) for item in measurements):
        raise ValueError("run.json measurements must contain only objects")
    successful = [m for m in measurements if m.get("exit_code") == 0 and m.get("integrity_ok") is True]
    failed = [m for m in measurements if not (m.get("exit_code") == 0 and m.get("integrity_ok") is True)]
    grouped: dict[str, list[dict[str, Any]]] = {}
    for item in measurements:
        trial_id = item.get("trial_id")
        if not isinstance(trial_id, str) or not trial_id:
            raise ValueError("every measurement must have a trial_id")
        grouped.setdefault(trial_id, []).append(item)
    evidence = manifest.get("dataset_evidence", {})
    trials = [evaluate_trial(config, trial_id, items, evidence, run_dir) for trial_id, items in sorted(grouped.items())]
    return {"successful": successful, "failed": failed, "trials": trials}


def quality_summary(run_dir: Path | None) -> list[dict[str, Any]]:
    if not run_dir:
        return []
    manifest = load_json(run_dir / "run.json")
    scores = manifest.get("scores", [])
    if not isinstance(scores, list):
        raise ValueError("run.json scores must be a list")
    measurements = manifest.get("measurements", [])
    derivations = manifest.get("derivations", [])
    scorer_by_kind = {"transcript": score_transcript, "diarization": score_diarization, "minutes": score_minutes}
    stage_by_kind = {"transcript": "transcription", "diarization": "diarization", "minutes": "minutes"}
    truth_by_kind = {"transcript": "transcript_path", "diarization": "diarization_path", "minutes": "minutes_truth_path"}
    seen: set[tuple[str, str]] = set()
    results: list[dict[str, Any]] = []
    for score in scores:
        if not isinstance(score, dict):
            raise ValueError("run.json scores must contain only objects")
        kind, trial_id = score.get("kind"), score.get("trial_id")
        issues: list[str] = []
        if not isinstance(kind, str) or kind not in scorer_by_kind or not isinstance(trial_id, str):
            issues.append("invalid kind or trial ID")
        else:
            key = (kind, trial_id)
            if key in seen:
                issues.append("duplicate score")
            seen.add(key)
        reference_path = Path(score.get("reference_path") or "")
        hypothesis_path = Path(score.get("hypothesis_path") or "")
        reference_hash = sha256_if_available(reference_path)
        hypothesis_hash = sha256_if_available(hypothesis_path)
        if reference_hash is None or reference_hash != score.get("reference_sha256"):
            issues.append("reference file changed or missing")
        if hypothesis_hash is None or hypothesis_hash != score.get("hypothesis_sha256"):
            issues.append("hypothesis file changed or missing")
        dataset_id = score.get("dataset_id")
        evidence = manifest.get("dataset_evidence", {}).get(dataset_id, {}) if isinstance(dataset_id, str) else {}
        if kind in truth_by_kind and reference_hash != evidence.get(truth_by_kind[kind], {}).get("sha256"):
            issues.append("reference does not match the frozen dataset")
        matching = [item for item in measurements if isinstance(item, dict)
                    and item.get("trial_id") == trial_id and item.get("stage") == stage_by_kind.get(kind)
                    and item.get("exit_code") == 0 and item.get("integrity_ok") is True]
        output_hashes: set[str] = set()
        if len(matching) != 1:
            issues.append("matching successful benchmark stage is missing")
        else:
            if matching[0].get("dataset_id") != dataset_id:
                issues.append("score dataset differs from the benchmark stage")
            for output in matching[0].get("output_files", []):
                if (isinstance(output, dict) and output.get("sha256")
                        and sha256_if_available(Path(output.get("path", ""))) == output["sha256"]):
                    output_hashes.add(output["sha256"])
            if kind == "transcript":
                for derivation in derivations:
                    if (isinstance(derivation, dict) and derivation.get("kind") == "whisper-json-to-transcript"
                            and derivation.get("trial_id") == trial_id
                            and derivation.get("input_sha256") in output_hashes
                            and sha256_if_available(Path(derivation.get("output_path", ""))) == derivation.get("output_sha256")):
                        output_hashes.add(derivation["output_sha256"])
        if hypothesis_hash not in output_hashes:
            issues.append("hypothesis provenance is broken")
        matches_path = score.get("matches_path")
        if kind == "minutes" and (not matches_path or not score.get("reviewer")):
            issues.append("minutes requires reviewer-provided matches")
        if matches_path and sha256_if_available(Path(matches_path)) != score.get("matches_sha256"):
            issues.append("reviewed matches changed or missing")
        terms_path = score.get("terms_path")
        frozen_terms = evidence.get("terms_path") if isinstance(evidence, dict) else None
        if kind == "transcript" and frozen_terms:
            if not isinstance(frozen_terms, dict) or not terms_path or score.get("terms_sha256") != frozen_terms.get("sha256"):
                issues.append("transcript terms do not match the frozen dataset lexicon")
        if terms_path:
            if kind != "transcript" or not frozen_terms or sha256_if_available(Path(terms_path)) != score.get("terms_sha256"):
                issues.append("transcript terms changed, missing, or are not frozen")
        if not issues:
            try:
                reference = load_json(reference_path)
                hypothesis = load_json(hypothesis_path)
                recalculated = scorer_by_kind[kind](
                    reference, hypothesis, load_json(Path(matches_path))
                ) if matches_path else scorer_by_kind[kind](
                    reference, hypothesis, load_json(Path(terms_path))
                ) if terms_path else scorer_by_kind[kind](reference, hypothesis)
                if recalculated != score.get("result"):
                    issues.append("stored score does not match recalculation")
                if kind == "transcript" and frozen_terms and recalculated.get("term_reference_occurrences") == 0:
                    issues.append("frozen terminology has no reference occurrences")
            except (OSError, ValueError, json.JSONDecodeError) as error:
                issues.append(f"score cannot be recalculated: {error}")
        results.append({
            "kind": kind, "trial_id": trial_id,
            "status": "verified_files" if not issues else "invalid",
            "metrics": score.get("result"), "issues": issues,
        })
    return results


def build_report(config_path: Path, run_dir: Path | None = None) -> dict[str, Any]:
    if run_dir and not (run_dir / "run.json").is_file():
        raise ValueError(f"run manifest does not exist: {run_dir / 'run.json'}")
    config = load_config(config_path)
    errors, warnings = validate_config(config, config_path)
    if run_dir:
        manifest = load_json(run_dir / "run.json")
        expected_hash = hashlib.sha256(config_path.read_bytes()).hexdigest()
        if manifest.get("config_sha256") != expected_hash:
            errors.append("run manifest configuration hash does not match --config")
    else:
        manifest = {}
    coverage = recording_coverage(config, run_dir)
    stability = stability_coverage(config, run_dir)
    benchmarks = benchmark_summary(config, run_dir)
    scores = quality_summary(run_dir)
    readiness_gaps: list[str] = []
    if errors or warnings:
        readiness_gaps.append("evaluation configuration has errors or unresolved prerequisites")
    if coverage["invalid"] or coverage["missing"]:
        readiness_gaps.append("the 32 recording conditions are not all reviewed with valid evidence")
    if stability["invalid"] or stability["missing"]:
        readiness_gaps.append("the eight OS-specific stability conditions are not all reviewed")
    if any(score["status"] != "verified_files" for score in scores):
        readiness_gaps.append("one or more quality scores have invalid provenance")
    devices = {item["id"]: item for item in config.get("devices", [])}
    final_device_ids = {
        item["id"] for item in config.get("devices", [])
        if item.get("required") and item.get("status") == "available" and item.get("memory_gib") == 16
    }
    scores_by_trial: dict[str, set[str]] = {}
    for score in scores:
        if score["status"] == "verified_files":
            scores_by_trial.setdefault(score["trial_id"], set()).add(score["kind"])
    expected_trials = {
        (platform, profile, run_kind, variant)
        for platform in ("macos", "windows")
        for profile in ("standard", "high-accuracy")
        for run_kind in ("cold", "warm")
        for variant in ("baseline", "terminology")
    }
    coverage_by_configuration: dict[tuple[str, str, str, str], set[tuple[str, str]]] = {}
    for trial in benchmarks["trials"]:
        profile = trial["profile"]
        eligible_statuses = {"timing_met", "timing_missed"} if profile == "standard" else {"measured"}
        if (trial["status"] in eligible_statuses
                and trial["device_id"] in final_device_ids
                and {"transcript", "diarization", "minutes"}.issubset(
                    scores_by_trial.get(trial["trial_id"], set()))):
            platform = devices.get(trial["device_id"], {}).get("platform")
            dataset_id = trial["dataset_id"]
            fingerprint = trial.get("model_fingerprint")
            axis = (platform, profile, trial["run_kind"], trial["variant"])
            if (isinstance(dataset_id, str) and isinstance(fingerprint, str)
                    and re.fullmatch(r"[0-9a-f]{64}", fingerprint) and axis in expected_trials):
                key = (dataset_id, platform, profile, fingerprint)
                coverage_by_configuration.setdefault(key, set()).add((trial["run_kind"], trial["variant"]))
    datasets = {key[0] for key in coverage_by_configuration}
    coverage_by_dataset: dict[str, set[tuple[str, str, str, str]]] = {}
    for dataset_id in datasets:
        covered_for_dataset: set[tuple[str, str, str, str]] = set()
        for platform in ("macos", "windows"):
            for profile in ("standard", "high-accuracy"):
                configurations = [axes for (data, os_name, level, _), axes in coverage_by_configuration.items()
                                  if data == dataset_id and os_name == platform and level == profile]
                best_axes = max(configurations, key=len, default=set())
                covered_for_dataset.update((platform, profile, run_kind, variant)
                                           for run_kind, variant in best_axes)
        coverage_by_dataset[dataset_id] = covered_for_dataset
    final_datasets = {
        item["id"] for item in config.get("datasets", [])
        if item.get("required") and item.get("speaker_count") == 10 and item.get("status") == "available"
    }
    best_dataset = max((name for name in coverage_by_dataset if name in final_datasets),
                       key=lambda name: len(coverage_by_dataset[name]), default=None)
    covered = coverage_by_dataset.get(best_dataset, set()) if best_dataset else set()
    if not final_datasets:
        readiness_gaps.append("a required 10-speaker evaluation dataset is not available")
    for platform, profile, run_kind, variant in sorted(expected_trials - covered):
        readiness_gaps.append(
            f"{platform}/{profile}/{run_kind}/{variant} lacks a complete timed trial "
            f"with three verified quality scores on one common dataset and model configuration")
    review = manifest.get("human_review", {})
    if (not isinstance(review, dict) or review.get("status") != "completed"
            or not isinstance(review.get("reviewer"), str) or not review["reviewer"].strip()
            or not isinstance(review.get("notes"), list)
            or not any(isinstance(note, str) and note.strip() for note in review["notes"])):
        readiness_gaps.append("human quality review is not completed")
    return {
        "schema_version": 1,
        "run": str(run_dir.resolve()) if run_dir else None,
        "configuration_errors": errors,
        "configuration_warnings": warnings,
        "devices_needed": [d["id"] for d in config.get("devices", []) if d.get("required") and d.get("status") != "available"],
        "datasets_needed": [d["id"] for d in config.get("datasets", []) if d.get("required") and d.get("status") != "available"],
        "pilot_datasets_needed": [d["id"] for d in config.get("datasets", []) if not d.get("required") and d.get("status") != "available"],
        "recordings": coverage,
        "stability": stability,
        "benchmarks": benchmarks,
        "scores": scores,
        "m1_06_ready_for_decision": not readiness_gaps,
        "readiness_gaps": readiness_gaps,
        "decision_note": "Readiness only means the evidence set can be reviewed; model choices and thresholds require a separate human decision record.",
    }


def render_markdown(report: dict[str, Any]) -> str:
    recording = report["recordings"]
    stability = report["stability"]
    benchmark = report["benchmarks"]
    scores = report["scores"]
    lines = [
        "# M1 技術検証レポート",
        "",
        f"- 実行記録: {report['run'] or '未指定'}",
        f"- 必須録音条件: {recording['reviewed_pass']}件合格 / {recording['reviewed_fail']}件不合格 / {len(recording['missing'])}件未検証（全{recording['expected']}件）",
        f"- 安定性条件: {stability['reviewed_pass']}件合格 / {stability['reviewed_fail']}件不合格 / {len(stability['missing'])}件未検証（全{stability['expected']}件）",
        f"- ベンチマーク: 成功{len(benchmark['successful'])}件 / 失敗{len(benchmark['failed'])}件",
        f"- 60分の標準試行: 時間目標達成{sum(item['status'] == 'timing_met' for item in benchmark['trials'])}件 / 未達{sum(item['status'] == 'timing_missed' for item in benchmark['trials'])}件",
        f"- 60分の高精度試行: 実測{sum(item['status'] == 'measured' for item in benchmark['trials'])}件（標準の時間目標は適用しない）",
        f"- 先行評価試行: 実測{sum(item['status'] == 'pilot_measured' for item in benchmark['trials'])}件（必須16GB・10話者の合否には算入しない）",
        f"- 先行評価データ: {'、'.join(report['pilot_datasets_needed']) if report['pilot_datasets_needed'] else '準備済み'}",
        f"- 比較不可・未完了試行: {sum(item['status'] not in ('timing_met', 'timing_missed', 'measured', 'pilot_measured') for item in benchmark['trials'])}件",
        f"- M1-06 構成決定: 未完了（証拠レビュー準備 {'完了' if report['m1_06_ready_for_decision'] else '未完了'}。モデルと合格値は別途人手で決定）",
        "",
    ]
    if benchmark["trials"]:
        lines.extend([
            "## 60分試行の時間比較",
            "",
            "| 試行 | データ | 端末 | 解析時間 | 議事録時間 | 判定 |",
            "|---|---|---|---:|---:|---|",
        ])
        for item in benchmark["trials"]:
            analysis = f"{item['analysis_seconds'] / 60:.2f}分" if item["analysis_seconds"] is not None else "未測定"
            minutes = f"{item['minutes_seconds'] / 60:.2f}分" if isinstance(item["minutes_seconds"], (int, float)) else "未測定"
            lines.append(f"| {item['trial_id']} | {item['dataset_id']} | {item['device_id']} | {analysis} | {minutes} | {item['status']} |")
        lines.append("")
    if scores:
        lines.extend(["## 品質スコア（ファイル整合のみ）", "",
                      "| 試行 | 種別 | 値 | 整合 |", "|---|---|---:|---|"])
        metric_by_kind = {"transcript": "cer", "diarization": "speaker_error_rate", "minutes": "recall"}
        for item in scores:
            metric = item["metrics"].get(metric_by_kind.get(item["kind"])) if isinstance(item["metrics"], dict) else None
            value = f"{metric:.4f}" if isinstance(metric, (int, float)) else "未算出"
            lines.append(f"| {item['trial_id']} | {item['kind']} | {value} | {item['status']} |")
        lines.append("")
    lines.extend(["## 不足・要確認", ""])
    issues = [f"設定: {item}" for item in report["configuration_errors"] + report["configuration_warnings"]]
    issues += [f"端末: {item}" for item in report["devices_needed"]]
    issues += [f"評価データ: {item}" for item in report["datasets_needed"]]
    issues += [f"先行評価データ: {item}" for item in report["pilot_datasets_needed"]]
    issues += [f"録音記録: {item}" for item in recording["invalid"]]
    issues += [f"未検証録音: {item}" for item in recording["missing"]]
    issues += [f"安定性記録: {item}" for item in stability["invalid"]]
    issues += [f"未検証安定性: {item}" for item in stability["missing"]]
    issues += [f"構成決定の証拠不足: {item}" for item in report["readiness_gaps"]]
    issues += [f"ベンチマーク失敗: {item.get('id', '?')}" for item in benchmark["failed"]]
    issues += [f"試行 {item['trial_id']}: {issue}" for item in benchmark["trials"] for issue in item["issues"]]
    issues += [f"品質 {item['trial_id']}/{item['kind']}: {issue}" for item in scores for issue in item["issues"]]
    if not benchmark["successful"]:
        issues.append("AIベンチマークの成功記録なし")
    if not scores:
        issues.append("品質スコアの添付なし")
    lines.extend(f"- {issue}" for issue in issues)
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, help="defaults to the run snapshot, or evaluation/config.toml without --run")
    parser.add_argument("--run", type=Path)
    parser.add_argument("--format", choices=("markdown", "json"), default="markdown")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        run_dir = args.run.resolve() if args.run else None
        config_path = args.config or (run_dir / "config.toml" if run_dir else Path(__file__).resolve().parents[1] / "evaluation/config.toml")
        report = build_report(config_path.resolve(), run_dir)
        output = json.dumps(report, ensure_ascii=False, indent=2) + "\n" if args.format == "json" else render_markdown(report)
        if args.output:
            args.output.write_text(output, encoding="utf-8")
        else:
            print(output, end="")
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.exit(2, f"error: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
