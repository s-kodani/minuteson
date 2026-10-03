#!/usr/bin/env python3
"""Inspect two recorded audio tracks without loading a full meeting into memory."""

from __future__ import annotations

import argparse
import array
import hashlib
import json
import math
import subprocess
import sys
from pathlib import Path
from typing import Any, Sequence


SAMPLE_RATE = 1000


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def media_info(path: Path) -> dict[str, Any]:
    completed = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_entries", "stream=codec_name,sample_rate,channels:format=duration", "-of", "json", str(path)],
        capture_output=True, text=True, check=False,
    )
    if completed.returncode:
        raise ValueError(f"ffprobe failed for {path}: {completed.stderr.strip()}")
    info = json.loads(completed.stdout)
    streams = info.get("streams", [])
    if not streams:
        raise ValueError(f"{path} has no audio stream")
    try:
        duration = float(info["format"]["duration"])
        sample_rate = int(streams[0]["sample_rate"])
        channels = int(streams[0]["channels"])
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError(f"{path} has incomplete audio metadata") from error
    if duration <= 0 or sample_rate <= 0 or channels <= 0:
        raise ValueError(f"{path} has invalid audio metadata")
    return {
        "duration_seconds": duration,
        "codec": streams[0].get("codec_name"),
        "sample_rate": sample_rate,
        "channels": channels,
    }


def read_window(path: Path, start_seconds: float, duration_seconds: float) -> list[float]:
    if start_seconds < 0 or not 0 < duration_seconds <= 20:
        raise ValueError("analysis window must start at or after zero and be at most 20 seconds")
    completed = subprocess.run(
        [
            "ffmpeg", "-nostdin", "-v", "error", "-ss", str(start_seconds), "-i", str(path),
            "-t", str(duration_seconds), "-vn", "-ac", "1", "-ar", str(SAMPLE_RATE),
            "-f", "s16le", "-acodec", "pcm_s16le", "-",
        ],
        capture_output=True, check=False,
    )
    if completed.returncode:
        raise ValueError(f"ffmpeg failed for {path}: {completed.stderr.decode('utf-8', errors='replace').strip()}")
    if len(completed.stdout) % 2:
        raise ValueError(f"{path} produced incomplete PCM samples")
    samples = array.array("h")
    samples.frombytes(completed.stdout)
    if sys.byteorder != "little":
        samples.byteswap()
    if not samples:
        raise ValueError(f"{path} has no audio in the selected window")
    return [sample / 32768.0 for sample in samples]


def inspect_track(paths: Sequence[Path]) -> dict[str, Any]:
    if not paths:
        raise ValueError("an audio track needs at least one segment")
    segments: list[dict[str, Any]] = []
    start = 0.0
    expected_format: tuple[Any, ...] | None = None
    for path in paths:
        info = media_info(path)
        current_format = (info["codec"], info["sample_rate"], info["channels"])
        if expected_format is not None and current_format != expected_format:
            raise ValueError(f"{path} has a different codec, sample rate, or channel count")
        expected_format = current_format
        segments.append({"file": str(path.resolve()), "start_seconds": start,
                         "end_seconds": start + info["duration_seconds"],
                         "duration_seconds": info["duration_seconds"],
                         "sha256": sha256_file(path)})
        start += info["duration_seconds"]
    return {"duration_seconds": start, "codec": expected_format[0],
            "sample_rate": expected_format[1], "channels": expected_format[2],
            "segments": segments}


def read_track_window(track: dict[str, Any], start_seconds: float, duration_seconds: float) -> list[float]:
    if start_seconds < 0 or duration_seconds <= 0 or start_seconds + duration_seconds > track["duration_seconds"] + 1e-6:
        raise ValueError("analysis window extends beyond the audio track")
    end_seconds = start_seconds + duration_seconds
    samples: list[float] = []
    for segment in track["segments"]:
        overlap_start = max(start_seconds, segment["start_seconds"])
        overlap_end = min(end_seconds, segment["end_seconds"])
        if overlap_end - overlap_start > 1e-6:
            samples.extend(read_window(
                Path(segment["file"]), overlap_start - segment["start_seconds"],
                overlap_end - overlap_start))
    if len(samples) < duration_seconds * SAMPLE_RATE * 0.99:
        raise ValueError("selected window produced too few samples across segments")
    return samples


def paths_from_windows_manifest(path: Path) -> tuple[list[Path], list[Path]]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema_version") != 1 or document.get("state") != "saved":
        raise ValueError("Windows capture manifest must have schema version 1 and saved state")
    base = path.resolve().parent
    tracks: list[list[Path]] = []
    for name in ("microphone", "system"):
        item = document.get(name)
        names = item.get("files") if isinstance(item, dict) else None
        if not isinstance(names, list) or not names:
            raise ValueError(f"capture manifest has no {name} segments")
        files: list[Path] = []
        for filename in names:
            if not isinstance(filename, str) or not filename:
                raise ValueError(f"capture manifest has an invalid {name} filename")
            resolved = (base / filename).resolve()
            if not resolved.is_relative_to(base) or not resolved.is_file():
                raise ValueError(f"{name} segment is missing or outside the manifest directory")
            files.append(resolved)
        tracks.append(files)
    return tracks[0], tracks[1]


def paths_from_macos_manifest(path: Path) -> tuple[list[Path], list[Path]]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schemaVersion") != 1 or document.get("state") != "saved":
        raise ValueError("macOS capture manifest must have schema version 1 and saved state")
    base = path.resolve().parent
    return [base / "microphone.caf"], [base / "system.caf"]


def offsets_from_windows_manifest(path: Path) -> tuple[float, float]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema_version") != 1 or document.get("state") != "saved":
        raise ValueError("Windows capture manifest must have schema version 1 and saved state")
    starts: list[int] = []
    for name in ("microphone", "system"):
        track = document.get(name)
        if not isinstance(track, dict):
            raise ValueError(f"capture manifest has no {name} timing")
        qpc = track.get("first_qpc_100ns")
        errors = track.get("timestamp_errors")
        discontinuities = track.get("discontinuities")
        if not isinstance(qpc, int) or isinstance(qpc, bool) or qpc <= 0:
            raise ValueError(f"capture manifest has no valid {name} first QPC timestamp")
        if (not isinstance(errors, int) or isinstance(errors, bool) or errors < 0
                or not isinstance(discontinuities, int) or isinstance(discontinuities, bool)
                or discontinuities < 0):
            raise ValueError(f"capture manifest has no {name} timestamp health counters")
        if errors or discontinuities:
            raise ValueError(
                f"{name} has timestamp errors or discontinuities; continuous QPC alignment is unsafe")
        starts.append(qpc)
    origin = min(starts)
    return ((starts[0] - origin) / 10_000_000,
            (starts[1] - origin) / 10_000_000)


def signal_metrics(samples: list[float]) -> dict[str, float | None]:
    peak = max(abs(value) for value in samples)
    rms = math.sqrt(sum(value * value for value in samples) / len(samples))
    return {
        "rms_dbfs": 20 * math.log10(rms) if rms else None,
        "peak_dbfs": 20 * math.log10(peak) if peak else None,
        "near_zero_fraction": sum(abs(value) < 0.001 for value in samples) / len(samples),
        "near_clip_fraction": sum(abs(value) >= 0.999 for value in samples) / len(samples),
    }


def correlation_at_lag(system: list[float], microphone: list[float], lag_samples: int) -> float | None:
    length = min(len(system), len(microphone))
    if abs(lag_samples) >= length - 1:
        return None
    if lag_samples >= 0:
        left = system[: length - lag_samples]
        right = microphone[lag_samples:length]
    else:
        left = system[-lag_samples:length]
        right = microphone[: length + lag_samples]
    count = len(left)
    sum_left = sum(left)
    sum_right = sum(right)
    numerator = sum(a * b for a, b in zip(left, right)) - sum_left * sum_right / count
    left_energy = sum(a * a for a in left) - sum_left * sum_left / count
    right_energy = sum(b * b for b in right) - sum_right * sum_right / count
    if left_energy <= 1e-12 or right_energy <= 1e-12:
        return None
    return numerator / math.sqrt(left_energy * right_energy)


def best_correlation(system: list[float], microphone: list[float], max_lag_ms: int) -> dict[str, float | None]:
    limit = round(max_lag_ms * SAMPLE_RATE / 1000)
    best_lag: int | None = None
    best_value: float | None = None
    for lag in range(-limit, limit + 1):
        value = correlation_at_lag(system, microphone, lag)
        if value is not None and (best_value is None or abs(value) > abs(best_value)):
            best_lag, best_value = lag, value
    reliable = best_value is not None and abs(best_value) >= 0.5
    return {
        "lag_ms": best_lag * 1000 / SAMPLE_RATE if reliable and best_lag is not None else None,
        "correlation": best_value,
    }


def analyze_pair(
    mic_path: Path | Sequence[Path],
    system_path: Path | Sequence[Path],
    window_start: float,
    window_seconds: float,
    max_lag_ms: int,
    calibration_signal_present: bool,
    microphone_start_offset_seconds: float = 0,
    system_start_offset_seconds: float = 0,
) -> dict[str, Any]:
    if (not math.isfinite(window_start) or not math.isfinite(window_seconds)
            or window_start < 0 or not 0 < window_seconds <= 20):
        raise ValueError("analysis window must have a non-negative start and at most 20 seconds")
    if max_lag_ms < 0 or max_lag_ms >= window_seconds * 1000 / 2:
        raise ValueError("max lag must be non-negative and less than half the window")
    if (not math.isfinite(microphone_start_offset_seconds)
            or not math.isfinite(system_start_offset_seconds)
            or min(microphone_start_offset_seconds, system_start_offset_seconds) < 0):
        raise ValueError("track start offsets must be non-negative")
    mic_paths = [mic_path] if isinstance(mic_path, Path) else list(mic_path)
    system_paths = [system_path] if isinstance(system_path, Path) else list(system_path)
    microphone = inspect_track(mic_paths)
    system = inspect_track(system_paths)
    common_start = max(microphone_start_offset_seconds, system_start_offset_seconds)
    common_end = min(
        microphone_start_offset_seconds + microphone["duration_seconds"],
        system_start_offset_seconds + system["duration_seconds"],
    )
    common_duration = common_end - common_start
    if window_start + window_seconds > common_duration:
        raise ValueError("analysis window extends beyond the tracks' common timeline")

    def analyze_window(start: float) -> dict[str, Any]:
        global_start = common_start + start
        mic_start = global_start - microphone_start_offset_seconds
        system_start = global_start - system_start_offset_seconds
        mic_samples = read_track_window(microphone, mic_start, window_seconds)
        system_samples = read_track_window(system, system_start, window_seconds)
        similarity = best_correlation(system_samples, mic_samples, max_lag_ms)
        return {
            "start_seconds": global_start,
            "duration_seconds": window_seconds,
            "microphone_file_start_seconds": mic_start,
            "system_file_start_seconds": system_start,
            "microphone": signal_metrics(mic_samples),
            "system": signal_metrics(system_samples),
            "cross_track": similarity,
        }

    early = analyze_window(window_start)
    late_start = common_duration - window_seconds
    late = analyze_window(late_start) if late_start - window_start >= window_seconds else None
    early_lag = early["cross_track"]["lag_ms"]
    late_lag = late["cross_track"]["lag_ms"] if late else None
    return {
        "schema_version": 1,
        "microphone_file": str(mic_paths[0].resolve()) if len(mic_paths) == 1 else None,
        "system_file": str(system_paths[0].resolve()) if len(system_paths) == 1 else None,
        "microphone_files": [str(path.resolve()) for path in mic_paths],
        "system_files": [str(path.resolve()) for path in system_paths],
        "microphone_metadata": microphone,
        "system_metadata": system,
        "duration_difference_seconds": microphone["duration_seconds"] - system["duration_seconds"],
        "common_overlap_seconds": common_duration,
        "microphone_start_offset_seconds": microphone_start_offset_seconds,
        "system_start_offset_seconds": system_start_offset_seconds,
        "calibration_signal_present": calibration_signal_present,
        "lag_interpretation": "system-to-microphone delay" if calibration_signal_present else "exploratory similarity only; no shared reference signal was confirmed",
        "early_window": early,
        "late_window": late,
        "estimated_drift_ms": late_lag - early_lag if calibration_signal_present and early_lag is not None and late_lag is not None else None,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--microphone", type=Path, action="append", help="repeat in segment order")
    parser.add_argument("--system", type=Path, action="append", help="repeat in segment order")
    parser.add_argument("--windows-manifest", type=Path, help="use both ordered tracks from a saved Windows capture manifest")
    parser.add_argument("--macos-manifest", type=Path, help="use both CAF tracks from a saved macOS capture manifest")
    parser.add_argument("--window-start", type=float, default=0)
    parser.add_argument("--window-seconds", type=float, default=6)
    parser.add_argument("--max-lag-ms", type=int, default=500)
    parser.add_argument("--calibration-signal-present", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.windows_manifest and args.macos_manifest:
            raise ValueError("choose only one capture manifest")
        if args.windows_manifest:
            if args.microphone or args.system:
                raise ValueError("--windows-manifest cannot be combined with direct track paths")
            microphone_paths, system_paths = paths_from_windows_manifest(args.windows_manifest)
            microphone_offset, system_offset = offsets_from_windows_manifest(args.windows_manifest)
        elif args.macos_manifest:
            if args.microphone or args.system:
                raise ValueError("--macos-manifest cannot be combined with direct track paths")
            microphone_paths, system_paths = paths_from_macos_manifest(args.macos_manifest)
            microphone_offset = system_offset = 0
        elif args.microphone and args.system:
            microphone_paths, system_paths = args.microphone, args.system
            microphone_offset = system_offset = 0
        else:
            raise ValueError("provide a capture manifest or at least one --microphone and --system")
        result = analyze_pair(
            microphone_paths, system_paths, args.window_start, args.window_seconds,
            args.max_lag_ms, args.calibration_signal_present,
            microphone_offset, system_offset,
        )
        manifest_path = args.windows_manifest or args.macos_manifest
        if manifest_path:
            result["source_manifest"] = str(manifest_path.resolve())
            result["source_manifest_sha256"] = sha256_file(manifest_path)
        if args.windows_manifest:
            result["timeline_alignment"] = "wasapi_qpc_100ns"
        else:
            result["timeline_alignment"] = "file_starts_assumed_equal"
        output = json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False) + "\n"
        if args.output:
            args.output.write_text(output, encoding="utf-8")
        else:
            print(output, end="")
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
