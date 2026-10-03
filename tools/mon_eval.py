#!/usr/bin/env python3
"""Reproducible, dependency-free evaluation tooling for MON milestone M1."""

from __future__ import annotations

import argparse
from collections import defaultdict
from contextlib import contextmanager
import hashlib
import itertools
import json
import math
import os
import platform
import re
import shutil
import subprocess
import sys
import time
import tomllib
import unicodedata
import uuid
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Any, Callable, Iterable, Iterator


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "evaluation" / "config.toml"
ALLOWED_PLATFORMS = {"macos", "windows"}
ALLOWED_SERVICES = {"meet", "zoom", "teams"}
ALLOWED_CAPTURE_MODES = {"system", "application"}
ALLOWED_OUTPUTS = {"headset", "speaker"}
ALLOWED_MINUTES_KINDS = {"decision", "todo", "open_issue", "topic", "summary"}


def load_json(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"{path}: JSON root must be an object")
    return data


@contextmanager
def manifest_lock(path: Path) -> Iterator[None]:
    lock_path = path.with_name(f".{path.name}.lock")
    deadline = time.monotonic() + 10
    while True:
        try:
            lock_path.mkdir()
            break
        except FileExistsError:
            if time.monotonic() >= deadline:
                raise ValueError(f"run manifest is locked: {lock_path}; check for an active writer before removing a stale lock")
            time.sleep(0.05)
    try:
        yield
    finally:
        lock_path.rmdir()


def update_manifest(path: Path, mutate: Callable[[dict[str, Any]], None]) -> None:
    with manifest_lock(path):
        manifest = load_json(path)
        mutate(manifest)
        temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
        try:
            with temporary.open("x", encoding="utf-8") as handle:
                json.dump(manifest, handle, ensure_ascii=False, indent=2)
                handle.write("\n")
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, path)
        finally:
            if temporary.exists():
                temporary.unlink()


def load_config(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def sha256_if_available(path: Path) -> str | None:
    try:
        return sha256_file(path)
    except OSError:
        return None


def normalize_text(value: str) -> str:
    value = unicodedata.normalize("NFKC", value).casefold()
    return "".join(char for char in value if not char.isspace() and not unicodedata.category(char).startswith("P"))


def normalize_words(value: str) -> list[str]:
    value = unicodedata.normalize("NFKC", value).casefold()
    value = "".join(" " if unicodedata.category(char).startswith("P") else char for char in value)
    return value.split()


def edit_distance(left: list[Any] | str, right: list[Any] | str) -> int:
    if len(left) < len(right):
        left, right = right, left
    previous = list(range(len(right) + 1))
    for row, lhs in enumerate(left, 1):
        current = [row]
        for column, rhs in enumerate(right, 1):
            current.append(min(current[-1] + 1, previous[column] + 1, previous[column - 1] + (lhs != rhs)))
        previous = current
    return previous[-1]


def ratio(errors: int | float, total: int | float) -> float | None:
    return errors / total if total else None


def transcript_text(document: dict[str, Any]) -> str:
    if isinstance(document.get("text"), str):
        return document["text"]
    utterances = document.get("utterances")
    if isinstance(utterances, list) and all(isinstance(item, dict) and isinstance(item.get("text"), str) for item in utterances):
        # Utterance boundaries are not annotated word boundaries. Inserting a
        # separator here would fabricate tokens for Japanese WER scoring.
        return "".join(item["text"] for item in utterances)
    raise ValueError("transcript requires text or utterances[].text")


def convert_whisper_json(document: dict[str, Any]) -> dict[str, Any]:
    segments = document.get("transcription")
    if not isinstance(segments, list):
        raise ValueError("whisper.cpp JSON requires transcription[]")
    utterances: list[dict[str, Any]] = []
    for index, segment in enumerate(segments):
        if not isinstance(segment, dict) or not isinstance(segment.get("offsets"), dict):
            raise ValueError(f"whisper.cpp transcription[{index}] lacks offsets")
        offsets = segment["offsets"]
        start_ms, end_ms = offsets.get("from"), offsets.get("to")
        if not isinstance(start_ms, int) or not isinstance(end_ms, int) or isinstance(start_ms, bool) or isinstance(end_ms, bool) or start_ms < 0 or end_ms <= start_ms:
            raise ValueError(f"whisper.cpp transcription[{index}] has invalid millisecond offsets")
        if not isinstance(segment.get("text"), str) or not segment["text"].strip():
            raise ValueError(f"whisper.cpp transcription[{index}] has empty text")
        utterances.append({
            "id": f"w{index + 1:06d}",
            "start_us": start_ms * 1000,
            "end_us": end_ms * 1000,
            "speaker": None,
            "text": segment["text"].strip(),
        })
    return {
        "schema_version": 1,
        "text": " ".join(item["text"] for item in utterances),
        "utterances": utterances,
        "source": "whisper.cpp segment-level JSON; token timestamps are ignored",
    }


def term_occurrences(normalized_text: str, normalized_term: str) -> int:
    prefix = r"(?<![a-z0-9])" if normalized_term[0].isascii() and normalized_term[0].isalnum() else ""
    suffix = r"(?![a-z0-9])" if normalized_term[-1].isascii() and normalized_term[-1].isalnum() else ""
    return len(re.findall(prefix + re.escape(normalized_term) + suffix, normalized_text))


def normalize_term_text(value: str) -> str:
    return " ".join(normalize_words(value))


def score_transcript(
    reference: dict[str, Any], hypothesis: dict[str, Any], terms: dict[str, Any] | None = None,
) -> dict[str, Any]:
    ref_text = transcript_text(reference)
    hyp_text = transcript_text(hypothesis)
    ref_chars = normalize_text(ref_text)
    hyp_chars = normalize_text(hyp_text)
    char_errors = edit_distance(ref_chars, hyp_chars)
    word_boundaries_available = (
        isinstance(reference.get("text"), str)
        and isinstance(hypothesis.get("text"), str)
        and any(char.isspace() for char in ref_text.strip())
    )
    ref_words = normalize_words(ref_text) if word_boundaries_available else []
    hyp_words = normalize_words(hyp_text) if word_boundaries_available else []
    word_errors = edit_distance(ref_words, hyp_words) if ref_words else None
    result = {
        "schema_version": 1,
        "cer": ratio(char_errors, len(ref_chars)),
        "character_errors": char_errors,
        "reference_characters": len(ref_chars),
        "wer": ratio(word_errors, len(ref_words)) if word_errors is not None else None,
        "word_errors": word_errors,
        "reference_words": len(ref_words) if word_boundaries_available else None,
        "normalization": "NFKC, case-fold, whitespace/punctuation removed for CER; punctuation replaced by spaces for WER",
    }
    if terms is not None:
        if not isinstance(reference.get("text"), str) or not isinstance(hypothesis.get("text"), str):
            raise ValueError("terminology scoring requires explicit text in reference and hypothesis")
        raw_terms = terms.get("terms")
        if terms.get("schema_version") != 1 or not isinstance(raw_terms, list) or not raw_terms:
            raise ValueError("terms requires schema_version 1 and a non-empty terms[]")
        normalized_terms: list[tuple[str, str]] = []
        seen: set[str] = set()
        for index, term in enumerate(raw_terms):
            if not isinstance(term, str) or not term.strip():
                raise ValueError(f"terms[{index}] must be non-empty text")
            normalized = normalize_term_text(term)
            if not normalized or normalized in seen:
                raise ValueError(f"terms[{index}] is empty or duplicated after normalization")
            seen.add(normalized)
            normalized_terms.append((term, normalized))
        details = []
        reference_total = hypothesis_total = correct_total = 0
        ref_term_text = normalize_term_text(ref_text)
        hyp_term_text = normalize_term_text(hyp_text)
        for term, normalized in normalized_terms:
            expected = term_occurrences(ref_term_text, normalized)
            actual = term_occurrences(hyp_term_text, normalized)
            correct = min(expected, actual)
            reference_total += expected
            hypothesis_total += actual
            correct_total += correct
            details.append({"term": term, "reference_occurrences": expected, "hypothesis_occurrences": actual, "matched_occurrences": correct})
        result.update({
            "term_recall": ratio(correct_total, reference_total),
            "term_precision": ratio(correct_total, hypothesis_total),
            "term_reference_occurrences": reference_total,
            "term_hypothesis_occurrences": hypothesis_total,
            "term_matched_occurrences": correct_total,
            "term_details": details,
            "term_method": "NFKC case-fold, punctuation-to-space whole-transcript occurrence counts; not time-aligned semantic matches",
        })
    return result


@dataclass(frozen=True)
class Segment:
    start: int
    end: int
    speaker: str


def parse_segments(document: dict[str, Any]) -> list[Segment]:
    raw = document.get("segments")
    if not isinstance(raw, list):
        raise ValueError("diarization requires segments[]")
    parsed: list[Segment] = []
    for index, item in enumerate(raw):
        if not isinstance(item, dict):
            raise ValueError(f"segments[{index}] must be an object")
        start, end, speaker = item.get("start_us"), item.get("end_us"), item.get("speaker")
        if not isinstance(start, int) or not isinstance(end, int) or isinstance(start, bool) or isinstance(end, bool) or start < 0 or end <= start:
            raise ValueError(f"segments[{index}] requires 0 <= start_us < end_us")
        if not isinstance(speaker, str) or not speaker:
            raise ValueError(f"segments[{index}].speaker must be non-empty")
        parsed.append(Segment(start, end, speaker))
    return parsed


def diarization_intervals(reference: list[Segment], hypothesis: list[Segment]) -> list[tuple[int, set[str], set[str]]]:
    events: dict[int, list[tuple[int, str, int]]] = defaultdict(list)
    for group, segments in enumerate((reference, hypothesis)):
        for item in segments:
            events[item.start].append((group, item.speaker, 1))
            events[item.end].append((group, item.speaker, -1))
    boundaries = sorted(events)
    counts: tuple[dict[str, int], dict[str, int]] = ({}, {})
    intervals: list[tuple[int, set[str], set[str]]] = []
    for position, start in enumerate(boundaries[:-1]):
        for group, speaker, change in events[start]:
            next_count = counts[group].get(speaker, 0) + change
            if next_count:
                counts[group][speaker] = next_count
            else:
                counts[group].pop(speaker, None)
        end = boundaries[position + 1]
        intervals.append((end - start, set(counts[0]), set(counts[1])))
    return intervals


def best_speaker_mapping(reference: list[Segment], hypothesis: list[Segment]) -> dict[str, str]:
    ref_speakers = sorted({item.speaker for item in reference})
    hyp_speakers = sorted({item.speaker for item in hypothesis})
    if not ref_speakers or not hyp_speakers:
        return {}
    overlap: dict[tuple[str, str], int] = {(h, r): 0 for h in hyp_speakers for r in ref_speakers}
    for duration, active_ref, active_hyp in diarization_intervals(reference, hypothesis):
        for hyp in active_hyp:
            for ref in active_ref:
                overlap[(hyp, ref)] += duration
    if len(ref_speakers) > 10 or len(hyp_speakers) > 10:
        raise ValueError("speaker mapping supports at most ten reference and hypothesis speakers")
    # Assign the smaller side to the larger side. This is O(n * 2^n * m),
    # rather than the factorial search that becomes impractical at ten speakers.
    small = hyp_speakers if len(hyp_speakers) <= len(ref_speakers) else ref_speakers
    large = ref_speakers if len(hyp_speakers) <= len(ref_speakers) else hyp_speakers
    hyp_is_small = len(hyp_speakers) <= len(ref_speakers)

    @lru_cache(maxsize=None)
    def assign(index: int, used_mask: int) -> tuple[int, tuple[int, ...]]:
        if index == len(small):
            return 0, ()
        best_score = -1
        best_indices: tuple[int, ...] = ()
        for large_index, large_speaker in enumerate(large):
            if used_mask & (1 << large_index):
                continue
            hyp, ref = (small[index], large_speaker) if hyp_is_small else (large_speaker, small[index])
            remainder, indices = assign(index + 1, used_mask | (1 << large_index))
            candidate = overlap[(hyp, ref)] + remainder
            if candidate > best_score:
                best_score = candidate
                best_indices = (large_index,) + indices
        return best_score, best_indices

    _, indices = assign(0, 0)
    if hyp_is_small:
        return {hyp: large[index] for hyp, index in zip(small, indices)}
    return {large[index]: ref for ref, index in zip(small, indices)}


def score_diarization(reference_doc: dict[str, Any], hypothesis_doc: dict[str, Any]) -> dict[str, Any]:
    reference = parse_segments(reference_doc)
    hypothesis = parse_segments(hypothesis_doc)
    mapping = best_speaker_mapping(reference, hypothesis)
    miss = false_alarm = confusion = reference_speaker_time = overlap_time = 0
    predicted_overlap_time = correctly_detected_overlap_time = 0
    for duration, ref, active_hyp in diarization_intervals(reference, hypothesis):
        hyp = {mapping.get(item, f"__unmapped__:{item}") for item in active_hyp}
        reference_speaker_time += duration * len(ref)
        if len(ref) > 1:
            overlap_time += duration
        if len(active_hyp) > 1:
            predicted_overlap_time += duration
            if len(ref) > 1:
                correctly_detected_overlap_time += duration
        correct = len(ref & hyp)
        miss += duration * max(0, len(ref) - len(hyp))
        false_alarm += duration * max(0, len(hyp) - len(ref))
        confusion += duration * max(0, min(len(ref), len(hyp)) - correct)
    errors = miss + false_alarm + confusion
    return {
        "schema_version": 1,
        "speaker_error_rate": ratio(errors, reference_speaker_time),
        "miss_us": miss,
        "false_alarm_us": false_alarm,
        "confusion_us": confusion,
        "reference_speaker_time_us": reference_speaker_time,
        "overlap_duration_us": overlap_time,
        "predicted_overlap_duration_us": predicted_overlap_time,
        "correctly_detected_overlap_duration_us": correctly_detected_overlap_time,
        "overlap_precision": ratio(correctly_detected_overlap_time, predicted_overlap_time),
        "overlap_recall": ratio(correctly_detected_overlap_time, overlap_time),
        "boundary_tolerance_us": 0,
        "reference_speakers": len({item.speaker for item in reference}),
        "hypothesis_speakers": len({item.speaker for item in hypothesis}),
        "speaker_mapping": mapping,
    }


def validate_minutes(document: dict[str, Any], label: str) -> list[str]:
    errors: list[str] = []
    items = document.get("items")
    if not isinstance(items, list):
        return [f"{label}.items must be an array"]
    seen: set[str] = set()
    for index, item in enumerate(items):
        prefix = f"{label}.items[{index}]"
        if not isinstance(item, dict):
            errors.append(f"{prefix} must be an object")
            continue
        item_id = item.get("id")
        if not isinstance(item_id, str) or not item_id:
            errors.append(f"{prefix}.id must be non-empty")
        elif item_id in seen:
            errors.append(f"{prefix}.id is duplicated")
        else:
            seen.add(item_id)
        if item.get("kind") not in ALLOWED_MINUTES_KINDS:
            errors.append(f"{prefix}.kind is invalid")
        if not isinstance(item.get("body"), str) or not item["body"].strip():
            errors.append(f"{prefix}.body must be non-empty")
        evidence = item.get("evidence_utterance_ids", [])
        if not isinstance(evidence, list) or not all(isinstance(value, str) and value for value in evidence):
            errors.append(f"{prefix}.evidence_utterance_ids must be an array of IDs")
        elif len(evidence) != len(set(evidence)):
            errors.append(f"{prefix}.evidence_utterance_ids contains duplicates")
        for field in ("assignee", "due"):
            value = item.get(field)
            if value is not None and (not isinstance(value, str) or not value.strip()):
                errors.append(f"{prefix}.{field} must be a non-empty string or null")
    return errors


def validate_dataset_documents(
    transcript: dict[str, Any],
    diarization: dict[str, Any],
    minutes: dict[str, Any],
    duration_seconds: float,
    expected_speakers: int,
    require_overlap: bool = False,
    require_minutes_coverage: bool = False,
) -> list[str]:
    errors: list[str] = []
    for label, document in (("transcript", transcript), ("diarization", diarization), ("minutes", minutes)):
        if document.get("schema_version") != 1:
            errors.append(f"{label}.schema_version must be 1")
    utterances = transcript.get("utterances")
    if not isinstance(utterances, list) or not utterances:
        errors.append("transcript.utterances must be a non-empty array")
        utterances = []
    utterance_ids: set[str] = set()
    transcript_speakers: set[str] = set()
    speaker_utterances: list[tuple[int, int, int, str]] = []
    max_end_us = int(duration_seconds * 1_000_000) + 1_000_000
    for index, item in enumerate(utterances):
        prefix = f"transcript.utterances[{index}]"
        if not isinstance(item, dict):
            errors.append(f"{prefix} must be an object")
            continue
        utterance_id = item.get("id")
        if not isinstance(utterance_id, str) or not utterance_id or utterance_id in utterance_ids:
            errors.append(f"{prefix}.id must be unique and non-empty")
        else:
            utterance_ids.add(utterance_id)
        start, end = item.get("start_us"), item.get("end_us")
        valid_interval = isinstance(start, int) and isinstance(end, int) and not isinstance(start, bool) and not isinstance(end, bool) and 0 <= start < end <= max_end_us
        if not valid_interval:
            errors.append(f"{prefix} has an invalid interval")
        speaker = item.get("speaker")
        if speaker is not None:
            if not isinstance(speaker, str) or not speaker:
                errors.append(f"{prefix}.speaker must be a non-empty ID or null")
            else:
                transcript_speakers.add(speaker)
                if valid_interval:
                    speaker_utterances.append((index, start, end, speaker))
        if not isinstance(item.get("text"), str) or not item["text"].strip():
            errors.append(f"{prefix}.text must be non-empty")
    if isinstance(transcript.get("text"), str) and utterances:
        composed = " ".join(item["text"] for item in utterances if isinstance(item, dict) and isinstance(item.get("text"), str))
        if normalize_text(transcript["text"]) != normalize_text(composed):
            errors.append("transcript.text differs from the ordered utterances")
    try:
        segments = parse_segments(diarization)
    except ValueError as error:
        errors.append(f"diarization: {error}")
        segments = []
    diarization_speakers = {item.speaker for item in segments}
    if len(diarization_speakers) != expected_speakers:
        errors.append(f"diarization has {len(diarization_speakers)} speakers; expected {expected_speakers}")
    if require_overlap and not any(len(active_ref) > 1 for _, active_ref, _ in diarization_intervals(segments, [])):
        errors.append("diarization has no simultaneous speech by distinct speakers despite contains_overlap=true")
    for index, segment in enumerate(segments):
        if segment.end > max_end_us:
            errors.append(f"diarization.segments[{index}] extends beyond audio duration")
    for speaker in sorted(transcript_speakers - diarization_speakers):
        errors.append(f"transcript speaker {speaker} has no diarization segment")
    for index, start, end, speaker in speaker_utterances:
        if speaker in diarization_speakers and not any(
            segment.speaker == speaker and start < segment.end and segment.start < end for segment in segments
        ):
            errors.append(f"transcript.utterances[{index}] has no overlapping diarization segment for speaker {speaker}")
    errors.extend(validate_minutes(minutes, "minutes"))
    minutes_items = minutes.get("items", []) if isinstance(minutes.get("items"), list) else []
    if require_minutes_coverage:
        valid_items = [item for item in minutes_items if isinstance(item, dict)]
        if not any(item.get("kind") == "decision" for item in valid_items):
            errors.append("minutes has no reference decision")
        if not any(item.get("kind") == "todo" and all(
            isinstance(item.get(field), str) and item[field].strip() for field in ("assignee", "due")
        ) for item in valid_items):
            errors.append("minutes has no reference todo with an explicit assignee and due date")
        if not any(item.get("kind") == "todo" and (item.get("assignee") is None or item.get("due") is None)
                   for item in valid_items):
            errors.append("minutes has no reference todo with an unconfirmed assignee or due date")
    for index, item in enumerate(minutes_items):
        if not isinstance(item, dict):
            continue
        if item.get("kind") in {"decision", "todo"} and not item.get("evidence_utterance_ids"):
            errors.append(f"minutes.items[{index}] has no evidence")
        for utterance_id in item.get("evidence_utterance_ids", []) if isinstance(item.get("evidence_utterance_ids"), list) else []:
            if utterance_id not in utterance_ids:
                errors.append(f"minutes.items[{index}] references unknown utterance {utterance_id}")
    return errors


def audio_metadata(path: Path) -> dict[str, Any]:
    completed = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration:stream=codec_name,sample_rate,channels", "-select_streams", "a:0", "-of", "json", str(path)],
        check=False, capture_output=True, text=True,
    )
    if completed.returncode:
        raise ValueError(f"ffprobe could not read {path}: {completed.stderr.strip()}")
    metadata = json.loads(completed.stdout)
    streams = metadata.get("streams", [])
    if not streams:
        raise ValueError(f"{path} has no audio stream")
    stream = streams[0]
    return {
        "duration_seconds": float(metadata.get("format", {}).get("duration", 0)),
        "codec": stream.get("codec_name"),
        "sample_rate": int(stream.get("sample_rate", 0)),
        "channels": int(stream.get("channels", 0)),
    }


def score_minutes(
    reference: dict[str, Any], hypothesis: dict[str, Any], matches: dict[str, Any] | None = None,
) -> dict[str, Any]:
    errors = validate_minutes(reference, "reference") + validate_minutes(hypothesis, "hypothesis")
    if errors:
        raise ValueError("; ".join(errors))
    ref_items = {item["id"]: item for item in reference["items"] if item["kind"] in {"decision", "todo"}}
    hyp_items = {item["id"]: item for item in hypothesis["items"] if item["kind"] in {"decision", "todo"}}
    if matches is None:
        pairs = [(item_id, item_id) for item_id in sorted(ref_items.keys() & hyp_items.keys())]
        matching_description = "identical item IDs; semantic equivalence requires human review"
    else:
        if matches.get("schema_version") != 1 or not isinstance(matches.get("matches"), list):
            raise ValueError("matches requires schema_version 1 and matches[]")
        pairs: list[tuple[str, str]] = []
        for index, pair in enumerate(matches["matches"]):
            if not isinstance(pair, dict) or not isinstance(pair.get("reference_id"), str) or not isinstance(pair.get("hypothesis_id"), str):
                raise ValueError(f"matches[{index}] requires reference_id and hypothesis_id")
            pairs.append((pair["reference_id"], pair["hypothesis_id"]))
        if len({ref for ref, _ in pairs}) != len(pairs) or len({hyp for _, hyp in pairs}) != len(pairs):
            raise ValueError("matches must be one-to-one")
        for ref_id, hyp_id in pairs:
            if ref_id not in ref_items or hyp_id not in hyp_items:
                raise ValueError(f"matches refers to an unknown decision or todo: {ref_id} -> {hyp_id}")
        matching_description = "reviewer-supplied one-to-one semantic matches"
    correct_pairs = [(ref_id, hyp_id) for ref_id, hyp_id in pairs
                     if ref_items[ref_id]["kind"] == hyp_items[hyp_id]["kind"]]
    matched_ref = {ref for ref, _ in correct_pairs}
    matched_hyp = {hyp for _, hyp in correct_pairs}
    true_positives = len(correct_pairs)
    precision = ratio(true_positives, len(hyp_items))
    recall = ratio(true_positives, len(ref_items))
    kind_scores = {}
    for kind in ("decision", "todo"):
        reference_count = sum(item["kind"] == kind for item in ref_items.values())
        hypothesis_count = sum(item["kind"] == kind for item in hyp_items.values())
        correct_count = sum(ref_items[ref_id]["kind"] == kind for ref_id, _ in correct_pairs)
        kind_scores[kind] = {
            "precision": ratio(correct_count, hypothesis_count),
            "recall": ratio(correct_count, reference_count),
            "reference_items": reference_count,
            "hypothesis_items": hypothesis_count,
            "correct_items": correct_count,
        }
    field_total = field_correct = evidence_total = predicted_evidence_total = evidence_correct = 0
    todo_total = todo_assignee_correct = todo_due_correct = 0
    mismatches: list[dict[str, Any]] = []
    for ref_id, hyp_id in pairs:
        ref, hyp = ref_items[ref_id], hyp_items[hyp_id]
        fields = {}
        for field in ("kind", "assignee", "due"):
            field_total += 1
            fields[field] = ref.get(field) == hyp.get(field)
            field_correct += int(fields[field])
        if ref["kind"] == "todo":
            todo_total += 1
            todo_assignee_correct += int(fields["assignee"])
            todo_due_correct += int(fields["due"])
        expected_evidence = set(ref.get("evidence_utterance_ids", []))
        actual_evidence = set(hyp.get("evidence_utterance_ids", []))
        evidence_total += len(expected_evidence)
        predicted_evidence_total += len(actual_evidence)
        evidence_correct += len(expected_evidence & actual_evidence)
        if not all(fields.values()) or expected_evidence != actual_evidence:
            mismatches.append({"reference_id": ref_id, "hypothesis_id": hyp_id, "fields": fields, "missing_evidence": sorted(expected_evidence - actual_evidence), "extra_evidence": sorted(actual_evidence - expected_evidence)})
    return {
        "schema_version": 1,
        "matching": matching_description,
        "precision": precision,
        "recall": recall,
        "kind_scores": kind_scores,
        "field_accuracy": ratio(field_correct, field_total),
        "todo_assignee_accuracy": ratio(todo_assignee_correct, todo_total),
        "todo_due_accuracy": ratio(todo_due_correct, todo_total),
        "evidence_precision": ratio(evidence_correct, predicted_evidence_total),
        "evidence_recall": ratio(evidence_correct, evidence_total),
        "matched_pairs": [{"reference_id": ref_id, "hypothesis_id": hyp_id} for ref_id, hyp_id in pairs],
        "misclassified_pairs": [{"reference_id": ref_id, "hypothesis_id": hyp_id,
                                  "reference_kind": ref_items[ref_id]["kind"], "hypothesis_kind": hyp_items[hyp_id]["kind"]}
                                 for ref_id, hyp_id in pairs if ref_items[ref_id]["kind"] != hyp_items[hyp_id]["kind"]],
        "missing_ids": sorted(ref_items.keys() - matched_ref),
        "unsupported_ids": sorted(hyp_items.keys() - matched_hyp),
        "mismatches": mismatches,
    }


def expand_recording_matrix(config: dict[str, Any]) -> list[dict[str, Any]]:
    matrix = config.get("recording_matrix", {})
    scenarios: list[dict[str, Any]] = []
    for platform_name, client_value, capture_mode, output in itertools.product(
        matrix.get("platforms", []), matrix.get("clients", []), matrix.get("capture_modes", []), matrix.get("outputs", [])
    ):
        if not isinstance(client_value, str) or ":" not in client_value:
            continue
        service, client = client_value.split(":", 1)
        scenarios.append({
            "id": f"{platform_name}-{service}-{client}-{capture_mode}-{output}",
            "platform": platform_name,
            "service": service,
            "client": client,
            "capture_mode": capture_mode,
            "output": output,
            "duration_seconds": matrix.get("duration_seconds"),
        })
    return scenarios


def validate_config(config: dict[str, Any], config_path: Path) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    if config.get("schema_version") != 1:
        errors.append("schema_version must be 1")
    thresholds = config.get("thresholds", {})
    for key in ("max_standard_processing_ratio", "max_minutes_processing_ratio", "max_recording_tail_loss_seconds", "max_speaker_count", "max_duration_seconds"):
        if not isinstance(thresholds.get(key), (int, float)) or thresholds[key] <= 0:
            errors.append(f"thresholds.{key} must be positive")
    devices = config.get("devices", [])
    device_ids: set[str] = set()
    for index, device in enumerate(devices):
        prefix = f"devices[{index}]"
        device_id = device.get("id")
        if not isinstance(device_id, str) or not device_id or device_id in device_ids:
            errors.append(f"{prefix}.id must be unique and non-empty")
        else:
            device_ids.add(device_id)
        if device.get("platform") not in ALLOWED_PLATFORMS:
            errors.append(f"{prefix}.platform must be macos or windows")
        if device.get("status") not in {"available", "needed", "retired"}:
            errors.append(f"{prefix}.status is invalid")
        if device.get("required") and device.get("status") != "available":
            warnings.append(f"{device_id}: required device is not available")
        if device.get("required") and device.get("power_source") not in {"AC", "battery"}:
            errors.append(f"{prefix}.power_source must be AC or battery")
        for key in ("os_version", "architecture", "cpu", "power_mode"):
            if not isinstance(device.get(key), str) or not device[key].strip():
                errors.append(f"{prefix}.{key} must be non-empty")
            elif device[key].strip().upper() == "TBD":
                warnings.append(f"{device_id}: {key} is TBD")
    datasets = config.get("datasets", [])
    dataset_ids: set[str] = set()
    for index, dataset in enumerate(datasets):
        prefix = f"datasets[{index}]"
        dataset_id = dataset.get("id")
        if not isinstance(dataset_id, str) or not dataset_id or dataset_id in dataset_ids:
            errors.append(f"{prefix}.id must be unique and non-empty")
        else:
            dataset_ids.add(dataset_id)
        if dataset.get("status") not in {"available", "needed", "retired"}:
            errors.append(f"{prefix}.status is invalid")
        if not isinstance(dataset.get("speaker_count"), int) or not 1 <= dataset["speaker_count"] <= thresholds.get("max_speaker_count", 0):
            errors.append(f"{prefix}.speaker_count exceeds configured range")
        if not isinstance(dataset.get("duration_seconds"), (int, float)) or not 0 < dataset["duration_seconds"] <= thresholds.get("max_duration_seconds", 0):
            errors.append(f"{prefix}.duration_seconds exceeds configured range")
        if dataset.get("required") and dataset.get("status") != "available":
            warnings.append(f"{dataset_id}: required dataset is not available")
        if dataset.get("required"):
            if dataset.get("speaker_count") != 10:
                errors.append(f"{prefix}.speaker_count must be 10 for the required evaluation dataset")
            for key in ("contains_overlap", "contains_english_terms"):
                if dataset.get(key) is not True:
                    errors.append(f"{prefix}.{key} must be true for the required evaluation dataset")
        if dataset.get("status") == "available":
            if not dataset.get("consent_confirmed"):
                errors.append(f"{prefix}.consent_confirmed must be true before use")
            audio_path = ROOT / dataset.get("audio_path", "")
            if not audio_path.is_file():
                errors.append(f"{prefix}.audio_path does not exist: {audio_path}")
            elif dataset.get("sha256") != sha256_file(audio_path):
                errors.append(f"{prefix}.sha256 does not match audio")
            else:
                try:
                    metadata = audio_metadata(audio_path)
                    expected_duration = float(dataset["duration_seconds"])
                    if abs(metadata["duration_seconds"] - expected_duration) > max(1.0, expected_duration * 0.01):
                        errors.append(f"{prefix}.duration_seconds differs from audio by more than 1%")
                    if metadata["codec"] != "flac":
                        errors.append(f"{prefix}.audio_path must contain FLAC audio")
                except (OSError, ValueError, json.JSONDecodeError) as error:
                    errors.append(f"{prefix}.audio_path cannot be inspected: {error}")
            documents: dict[str, dict[str, Any]] = {}
            for key in ("transcript_path", "diarization_path", "minutes_truth_path"):
                path = ROOT / dataset.get(key, "")
                if not path.is_file():
                    errors.append(f"{prefix}.{key} does not exist: {path}")
                else:
                    try:
                        documents[key] = load_json(path)
                    except (OSError, ValueError, json.JSONDecodeError) as error:
                        errors.append(f"{prefix}.{key} is invalid: {error}")
            terms_path = ROOT / dataset.get("terms_path", "")
            if dataset.get("required") and not dataset.get("terms_path"):
                errors.append(f"{prefix}.terms_path is required for terminology comparison")
            elif dataset.get("terms_path"):
                if not terms_path.is_file():
                    errors.append(f"{prefix}.terms_path does not exist: {terms_path}")
                else:
                    try:
                        if "transcript_path" in documents:
                            term_result = score_transcript(documents["transcript_path"], {"text": ""}, load_json(terms_path))
                            if term_result["term_reference_occurrences"] == 0:
                                errors.append(f"{prefix}.terms_path has no occurrences in the reference transcript")
                    except (OSError, ValueError, json.JSONDecodeError) as error:
                        errors.append(f"{prefix}.terms_path is invalid: {error}")
            if len(documents) == 3:
                errors.extend(f"{prefix}: {error}" for error in validate_dataset_documents(
                    documents["transcript_path"], documents["diarization_path"],
                    documents["minutes_truth_path"], float(dataset["duration_seconds"]), int(dataset["speaker_count"]),
                    require_overlap=dataset.get("contains_overlap") is True,
                    require_minutes_coverage=dataset.get("required") is True,
                ))
    matrix = config.get("recording_matrix", {})
    if not isinstance(matrix, dict):
        errors.append("recording_matrix must be an object")
        matrix = {}
    expected_axes = {
        "platforms": ALLOWED_PLATFORMS,
        "clients": {"meet:chrome", "meet:edge", "zoom:desktop", "teams:desktop"},
        "capture_modes": ALLOWED_CAPTURE_MODES,
        "outputs": ALLOWED_OUTPUTS,
    }
    for key, expected in expected_axes.items():
        actual = matrix.get(key)
        if not isinstance(actual, list) or set(actual) != expected:
            errors.append(f"recording_matrix.{key} must contain exactly {sorted(expected)}")
    duration = matrix.get("duration_seconds")
    if not isinstance(duration, (int, float)) or duration <= 0:
        errors.append("recording_matrix.duration_seconds must be positive")
    scenarios = expand_recording_matrix(config)
    scenario_ids: set[str] = set()
    for index, scenario in enumerate(scenarios):
        prefix = f"recording_scenarios[{index}]"
        scenario_id = scenario.get("id")
        if not isinstance(scenario_id, str) or not scenario_id or scenario_id in scenario_ids:
            errors.append(f"{prefix}.id must be unique and non-empty")
        else:
            scenario_ids.add(scenario_id)
        if scenario.get("platform") not in ALLOWED_PLATFORMS or scenario.get("service") not in ALLOWED_SERVICES or scenario.get("capture_mode") not in ALLOWED_CAPTURE_MODES or scenario.get("output") not in ALLOWED_OUTPUTS:
            errors.append(f"{prefix} has an invalid enum value")
    for required_platform in ALLOWED_PLATFORMS:
        if not any(item.get("platform") == required_platform and item.get("required") for item in devices):
            errors.append(f"missing required {required_platform} device")
        if not any(item.get("platform") == required_platform and item.get("required") and item.get("memory_gib") == 16 for item in devices):
            errors.append(f"missing required 16 GiB {required_platform} performance device")
        if not any(item.get("platform") == required_platform for item in scenarios):
            errors.append(f"missing {required_platform} recording scenario")
    if len(scenarios) != 32:
        errors.append(f"recording matrix must expand to 32 scenarios, got {len(scenarios)}")
    stability = config.get("stability_scenarios", [])
    required_stability = {"four-hour-recording", "device-disconnect-reconnect", "forced-termination-recovery", "explicit-sleep-resume"}
    actual_stability = {item.get("id") for item in stability if isinstance(item, dict)}
    if actual_stability != required_stability:
        errors.append(f"stability_scenarios must contain exactly {sorted(required_stability)}")
    for index, scenario in enumerate(stability):
        if not isinstance(scenario, dict) or set(scenario.get("platforms", [])) != ALLOWED_PLATFORMS:
            errors.append(f"stability_scenarios[{index}] must target both platforms")
    return errors, warnings


def command_doctor(_: argparse.Namespace) -> int:
    tools = {name: shutil.which(name) for name in ("ffmpeg", "ffprobe", "git", "python3", "cargo", "node", "npm")}
    payload = {
        "schema_version": 1,
        "captured_at_utc_ms": int(time.time() * 1000),
        "platform": platform.system(),
        "platform_release": platform.release(),
        "platform_version": platform.version(),
        "architecture": platform.machine(),
        "python": platform.python_version(),
        "tools": tools,
    }
    try:
        payload["host_facts"] = host_facts()
    except (OSError, ValueError, KeyError, IndexError, TypeError, subprocess.CalledProcessError) as error:
        payload["host_facts_error"] = str(error)
    print(json.dumps(payload, ensure_ascii=False, indent=2))
    return 0 if tools["ffmpeg"] and tools["ffprobe"] else 2


def command_validate(args: argparse.Namespace) -> int:
    path = Path(args.config).resolve()
    errors, warnings = validate_config(load_config(path), path)
    for warning in warnings:
        print(f"WARN: {warning}")
    for error in errors:
        print(f"ERROR: {error}")
    print(f"validation: {len(errors)} error(s), {len(warnings)} warning(s)")
    if args.strict and warnings:
        return 2
    return 1 if errors else 0


def command_new_run(args: argparse.Namespace) -> int:
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}", args.name):
        raise ValueError("run name must contain only letters, numbers, dot, underscore, or hyphen")
    config_path = Path(args.config).resolve()
    config = load_config(config_path)
    errors, warnings = validate_config(config, config_path)
    if errors:
        raise ValueError("configuration has errors; run validate first")
    timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    run_dir = ROOT / "evaluation" / "runs" / f"{timestamp}-{args.name}"
    run_dir.mkdir(parents=True, exist_ok=False)
    config_bytes = config_path.read_bytes()
    (run_dir / "config.toml").write_bytes(config_bytes)
    try:
        commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        commit = None
    manifest = {
        "schema_version": 1,
        "run_id": run_dir.name,
        "created_at_utc_ms": int(time.time() * 1000),
        "git_commit": commit,
        "config_sha256": hashlib.sha256(config_bytes).hexdigest(),
        "warnings_at_creation": warnings,
        "dataset_evidence": {
            dataset["id"]: {
                key: {
                    "path": str((ROOT / dataset[key]).resolve()),
                    "sha256": sha256_file(ROOT / dataset[key]),
                }
                for key in ("audio_path", "transcript_path", "diarization_path", "minutes_truth_path", "terms_path") if dataset.get(key)
            }
            for dataset in config.get("datasets", []) if dataset.get("status") == "available"
        },
        "measurements": [],
        "derivations": [],
        "scores": [],
        "human_review": {"status": "pending", "reviewer": None, "notes": []},
    }
    (run_dir / "run.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(run_dir)
    return 0


def command_list_scenarios(args: argparse.Namespace) -> int:
    config = load_config(Path(args.config).resolve())
    scenarios = expand_recording_matrix(config)
    if args.format == "json":
        print(json.dumps(scenarios, ensure_ascii=False, indent=2))
    else:
        print("id,platform,service,client,capture_mode,output,duration_seconds")
        for item in scenarios:
            print(",".join(str(item[key]) for key in ("id", "platform", "service", "client", "capture_mode", "output", "duration_seconds")))
    return 0


def process_rss_kib(process: subprocess.Popen[bytes]) -> int | None:
    if sys.platform == "win32":
        try:
            import ctypes
            from ctypes import wintypes

            class ProcessMemoryCounters(ctypes.Structure):
                _fields_ = [
                    ("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
                    ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
                    ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                    ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                    ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t),
                ]
            counters = ProcessMemoryCounters()
            counters.cb = ctypes.sizeof(counters)
            handle = wintypes.HANDLE(int(process._handle))  # type: ignore[attr-defined]
            if ctypes.windll.psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                return int(counters.PeakWorkingSetSize // 1024)
        except (AttributeError, OSError, ValueError):
            return None
        return None
    try:
        result = subprocess.run(["ps", "-o", "rss=", "-p", str(process.pid)], check=False, capture_output=True, text=True)
        value = result.stdout.strip()
        return int(value) if value else None
    except (OSError, ValueError):
        return None


def resolve_executable(command_name: str, cwd: str | None) -> Path:
    candidate = Path(command_name)
    if candidate.is_absolute():
        return candidate.resolve()
    if candidate.parent != Path(".") or command_name.startswith("."):
        return ((Path(cwd) if cwd else Path.cwd()) / candidate).resolve()
    located = shutil.which(command_name)
    if located is None:
        raise ValueError(f"benchmark executable was not found: {command_name}")
    return Path(located).resolve()


def host_meets_required_os(facts: dict[str, Any]) -> bool:
    if facts.get("platform") == "macos":
        major = facts.get("os_major")
        return type(major) is int and major >= 15
    if facts.get("platform") == "windows":
        build = facts.get("os_build")
        product_type = facts.get("os_product_type")
        return type(build) is int and build >= 26100 and product_type == 1
    return False


def host_facts() -> dict[str, Any]:
    system = platform.system()
    current_platform = {"Darwin": "macos", "Windows": "windows"}.get(system, system.lower())
    architecture = platform.machine().lower()
    if architecture in {"amd64", "x64"}:
        architecture = "x86_64"
    if current_platform == "macos":
        mac_version = platform.mac_ver()[0]
        os_major = int(mac_version.split(".")[0]) if re.fullmatch(r"\d+(?:\.\d+)*", mac_version) else None
        os_build = None
        os_product_type = None
        hardware = json.loads(subprocess.check_output(
            ["system_profiler", "SPHardwareDataType", "-json"], text=True, stderr=subprocess.DEVNULL,
        ))["SPHardwareDataType"][0]
        cpu_model = hardware.get("chip_type") or hardware.get("processor_name")
        try:
            total_bytes = int(subprocess.check_output(
                ["sysctl", "-n", "hw.memsize"], text=True, stderr=subprocess.DEVNULL).strip())
            memory_source = "sysctl_hw_memsize"
        except (OSError, subprocess.CalledProcessError, ValueError):
            nominal = hardware["physical_memory"]
            match = re.fullmatch(r"(\d+(?:\.\d+)?) GB", nominal)
            if not match:
                raise ValueError("could not parse macOS physical memory")
            total_bytes = round(float(match.group(1)) * (1024 ** 3))
            memory_source = "system_profiler_nominal"
        try:
            power_output = subprocess.check_output(["pmset", "-g", "batt"], text=True, stderr=subprocess.DEVNULL)
            match = re.search(r"Now drawing from '([^']+)'", power_output)
            power_source = "AC" if match and match.group(1) == "AC Power" else "battery" if match and match.group(1) == "Battery Power" else None
        except (OSError, subprocess.CalledProcessError):
            power_source = None
    elif current_platform == "windows":
        import ctypes

        windows_version = sys.getwindowsversion()
        os_major = None
        os_build = windows_version.build
        os_product_type = windows_version.product_type

        class MemoryStatus(ctypes.Structure):
            _fields_ = [
                ("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong),
                ("ullTotalPhys", ctypes.c_ulonglong), ("ullAvailPhys", ctypes.c_ulonglong),
                ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong),
                ("ullAvailExtendedVirtual", ctypes.c_ulonglong),
            ]
        status = MemoryStatus()
        status.dwLength = ctypes.sizeof(status)
        if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            raise ValueError("could not read Windows physical memory")
        total_bytes = status.ullTotalPhys
        memory_source = "GlobalMemoryStatusEx"
        try:
            cpu_output = subprocess.check_output(
                ["powershell", "-NoProfile", "-Command", "(Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name)"],
                text=True, stderr=subprocess.DEVNULL,
            )
            cpu_model = cpu_output.strip() or None
        except (OSError, subprocess.CalledProcessError):
            cpu_model = None

        class SystemPowerStatus(ctypes.Structure):
            _fields_ = [
                ("ACLineStatus", ctypes.c_ubyte), ("BatteryFlag", ctypes.c_ubyte),
                ("BatteryLifePercent", ctypes.c_ubyte), ("SystemStatusFlag", ctypes.c_ubyte),
                ("BatteryLifeTime", ctypes.c_ulong), ("BatteryFullLifeTime", ctypes.c_ulong),
            ]
        power_status = SystemPowerStatus()
        if ctypes.windll.kernel32.GetSystemPowerStatus(ctypes.byref(power_status)):
            power_source = {0: "battery", 1: "AC"}.get(power_status.ACLineStatus)
        else:
            power_source = None
    else:
        os_major = None
        os_build = None
        os_product_type = None
        total_bytes = os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
        memory_source = "sysconf"
        cpu_model = platform.processor() or None
        power_source = None
    return {
        "platform": current_platform,
        "architecture": architecture,
        "physical_memory_bytes": total_bytes,
        "memory_gib_rounded": round(total_bytes / (1024 ** 3)),
        "memory_source": memory_source,
        "cpu_model": cpu_model,
        "power_source": power_source,
        "os_version": platform.platform(),
        "os_major": os_major,
        "os_build": os_build,
        "os_product_type": os_product_type,
    }


def command_benchmark(args: argparse.Namespace) -> int:
    run_dir = Path(args.run).resolve()
    manifest_path = run_dir / "run.json"
    if not manifest_path.is_file():
        raise ValueError(f"run manifest does not exist: {manifest_path}")
    config_path = run_dir / "config.toml"
    if not config_path.is_file():
        raise ValueError(f"run config snapshot does not exist: {config_path}")
    config = load_config(config_path)
    devices = {item.get("id"): item for item in config.get("devices", [])}
    datasets = {item.get("id"): item for item in config.get("datasets", [])}
    device = devices.get(args.device_id)
    dataset = datasets.get(args.dataset_id)
    if not device or device.get("status") != "available":
        raise ValueError("--device-id must name an available device in the run config")
    facts = host_facts()
    if (device.get("platform") != facts["platform"] or
            device.get("architecture") != facts["architecture"] or
            device.get("memory_gib") != facts["memory_gib_rounded"]):
        raise ValueError("selected device does not match this host's platform, architecture, and physical memory")
    if device.get("required") and (
        not isinstance(facts.get("cpu_model"), str)
        or str(device.get("cpu") or "").strip().casefold() != facts["cpu_model"].strip().casefold()
        or device.get("power_source") != facts.get("power_source")
    ):
        raise ValueError("required device CPU model or power source differs from the host")
    if device.get("required") and not host_meets_required_os(facts):
        raise ValueError("required device does not meet the macOS 15 or Windows 11 24H2 minimum")
    if not dataset or dataset.get("status") != "available" or not dataset.get("consent_confirmed"):
        raise ValueError("--dataset-id must name an available, consented dataset in the run config")
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}", args.trial_id):
        raise ValueError("--trial-id must contain only letters, numbers, dot, underscore, or hyphen")
    command = list(args.executable)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        raise ValueError("benchmark requires a command after --")
    executable_path = resolve_executable(command[0], args.cwd)
    if not executable_path.is_file():
        raise ValueError(f"benchmark executable is not a file: {executable_path}")
    executable_file = {"path": str(executable_path), "sha256": sha256_file(executable_path)}
    command[0] = str(executable_path)
    input_path = Path(args.input).resolve()
    if not input_path.is_file():
        raise ValueError(f"input does not exist: {input_path}")
    dataset_duration = float(dataset["duration_seconds"])
    input_duration = args.input_duration if args.input_duration is not None else dataset_duration
    if not math.isfinite(input_duration) or input_duration <= 0 or abs(input_duration - dataset_duration) > max(1.0, dataset_duration * 0.01):
        raise ValueError("--input-duration must match the configured dataset duration within 1%")
    model_paths = [Path(value).resolve() for value in args.model_file]
    settings_value = getattr(args, "engine_settings_file", None)
    settings_path = Path(settings_value).resolve() if settings_value else None
    if device.get("required") and settings_path is None:
        raise ValueError("required device benchmark needs --engine-settings-file")
    if settings_path is not None:
        if not settings_path.is_file():
            raise ValueError("--engine-settings-file does not exist")
        settings_document = load_json(settings_path)
        if (settings_document.get("schema_version") != 1
                or settings_document.get("engine") != args.engine
                or not isinstance(settings_document.get("settings"), dict)
                or not settings_document["settings"]):
            raise ValueError("engine settings must contain schema_version 1, the selected engine, and non-empty settings")
        settings_file = {"path": str(settings_path), "sha256": sha256_file(settings_path)}
    else:
        settings_file = None
    dependency_paths = [Path(value).resolve() for value in getattr(args, "dependency_file", [])]
    output_paths = [Path(value).resolve() for value in args.output_file]
    if args.stage in {"transcription", "diarization", "minutes"} and not model_paths:
        raise ValueError(f"--model-file is required for {args.stage}")
    if any(not path.is_file() for path in model_paths):
        raise ValueError("every --model-file must exist")
    if len(set(model_paths)) != len(model_paths):
        raise ValueError("duplicate --model-file paths are not allowed")
    if len(set(dependency_paths)) != len(dependency_paths) or input_path in dependency_paths:
        raise ValueError("duplicate --dependency-file paths or primary input are not allowed")
    if any(not path.is_file() for path in dependency_paths):
        raise ValueError("every --dependency-file must exist")
    if len(set(output_paths)) != len(output_paths):
        raise ValueError("duplicate --output-file paths are not allowed")
    if any(path.exists() for path in output_paths):
        raise ValueError("every --output-file must name a new path, not an existing file")
    manifest = load_json(manifest_path)
    config_sha256 = sha256_file(config_path)
    if manifest.get("config_sha256") != config_sha256:
        raise ValueError("run manifest configuration hash does not match its snapshot")
    measurements = manifest.get("measurements", [])
    if not isinstance(measurements, list) or any(not isinstance(item, dict) for item in measurements):
        raise ValueError("run measurements must be a list of objects")
    if any(item.get("trial_id") == args.trial_id and item.get("stage") == args.stage for item in measurements):
        raise ValueError("this trial already has a measurement for the selected stage")
    input_sha256 = sha256_file(input_path)
    if args.stage == "audio-preparation":
        frozen_audio = manifest.get("dataset_evidence", {}).get(args.dataset_id, {}).get("audio_path", {})
        if input_sha256 != frozen_audio.get("sha256"):
            raise ValueError("audio-preparation input does not match the frozen dataset audio")
    dependency_files = [{"path": str(path), "sha256": sha256_file(path)} for path in dependency_paths]
    if args.stage == "transcription" and dataset.get("terms_path"):
        frozen_terms = manifest.get("dataset_evidence", {}).get(args.dataset_id, {}).get("terms_path")
        if not isinstance(frozen_terms, dict) or not isinstance(frozen_terms.get("sha256"), str):
            raise ValueError("transcription requires a frozen dataset terminology list")
        has_terms_dependency = any(item["sha256"] == frozen_terms["sha256"] for item in dependency_files)
        if args.variant == "terminology" and not has_terms_dependency:
            raise ValueError("terminology trial must declare the frozen terms as a --dependency-file")
        if args.variant == "baseline" and has_terms_dependency:
            raise ValueError("baseline transcription must not declare the frozen terms as a dependency")
    model_files = [{"path": str(path), "sha256": sha256_file(path), "size_bytes": path.stat().st_size} for path in model_paths]
    measurement_id = f"{args.trial_id}-{args.stage}-{uuid.uuid4().hex[:16]}"
    stdout_path = run_dir / f"{measurement_id}.stdout.log"
    stderr_path = run_dir / f"{measurement_id}.stderr.log"
    started_at = int(time.time() * 1000)
    started = time.perf_counter()
    peak_rss_kib: int | None = None
    with stdout_path.open("xb") as stdout_handle, stderr_path.open("xb") as stderr_handle:
        try:
            process = subprocess.Popen(command, cwd=args.cwd, stdout=stdout_handle, stderr=stderr_handle)
        except OSError as error:
            raise ValueError(f"cannot start benchmark command: {error}") from error
        if sys.platform == "darwin":
            while True:
                completed_pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                if completed_pid:
                    process.returncode = os.waitstatus_to_exitcode(status)
                    peak_rss_kib = max(1, round(usage.ru_maxrss / 1024))
                    break
                time.sleep(0.05)
        else:
            while process.poll() is None:
                rss = process_rss_kib(process)
                if rss is not None:
                    peak_rss_kib = max(peak_rss_kib or 0, rss)
                time.sleep(0.05)
        exit_code = process.returncode
    elapsed = time.perf_counter() - started
    try:
        end_facts = host_facts()
        end_facts_error = None
    except (OSError, ValueError, KeyError, IndexError, TypeError, subprocess.CalledProcessError) as error:
        end_facts = {}
        end_facts_error = str(error)
    required_host_stable = not device.get("required") or (
        end_facts.get("platform") == facts.get("platform")
        and end_facts.get("architecture") == facts.get("architecture")
        and end_facts.get("memory_gib_rounded") == facts.get("memory_gib_rounded")
        and end_facts.get("cpu_model") == facts.get("cpu_model")
        and end_facts.get("power_source") == facts.get("power_source")
        and end_facts.get("os_version") == facts.get("os_version")
        and end_facts.get("os_major") == facts.get("os_major")
        and end_facts.get("os_build") == facts.get("os_build")
        and end_facts.get("os_product_type") == facts.get("os_product_type")
    )
    input_changed = sha256_if_available(input_path) != input_sha256
    for entry, path in zip(dependency_files, dependency_paths):
        entry["changed_during_run"] = sha256_if_available(path) != entry["sha256"]
    for entry, path in zip(model_files, model_paths):
        entry["changed_during_run"] = sha256_if_available(path) != entry["sha256"]
    executable_file["changed_during_run"] = sha256_if_available(executable_path) != executable_file["sha256"]
    if settings_file is not None and settings_path is not None:
        settings_file["changed_during_run"] = sha256_if_available(settings_path) != settings_file["sha256"]
    output_files = [
        {"path": str(path), "sha256": sha256_if_available(path),
         "size_bytes": path.stat().st_size if path.is_file() else None}
        for path in output_paths
    ]
    config_changed = sha256_if_available(config_path) != config_sha256
    integrity_ok = (not input_changed and not config_changed
                    and required_host_stable
                    and not any(entry["changed_during_run"] for entry in dependency_files)
                    and not any(entry["changed_during_run"] for entry in model_files)
                    and not executable_file["changed_during_run"]
                    and (settings_file is None or not settings_file["changed_during_run"])
                    and not any(entry["sha256"] is None for entry in output_files))
    measurement = {
        "id": measurement_id,
        "stage": args.stage,
        "engine": args.engine,
        "profile": args.profile,
        "run_kind": args.run_kind,
        "variant": args.variant,
        "trial_id": args.trial_id,
        "device_id": args.device_id,
        "host_facts": facts,
        "host_facts_at_end": end_facts,
        "host_facts_at_end_error": end_facts_error,
        "required_host_stable": required_host_stable,
        "dataset_id": args.dataset_id,
        "dataset_audio_sha256": dataset["sha256"],
        "model_files": model_files,
        "executable_file": executable_file,
        "engine_settings_file": settings_file,
        "output_files": output_files,
        "started_at_utc_ms": started_at,
        "command": command,
        "cwd": str(Path(args.cwd).resolve()) if args.cwd else str(ROOT),
        "input_path": str(input_path),
        "input_sha256": input_sha256,
        "input_changed_during_run": input_changed,
        "dependency_files": dependency_files,
        "config_changed_during_run": config_changed,
        "integrity_ok": integrity_ok,
        "input_duration_seconds": input_duration,
        "wall_time_seconds": elapsed,
        "processing_ratio": elapsed / input_duration,
        "peak_rss_kib": peak_rss_kib,
        "exit_code": exit_code,
        "stdout_log": stdout_path.name,
        "stderr_log": stderr_path.name,
        "stdout_sha256": sha256_file(stdout_path),
        "stderr_sha256": sha256_file(stderr_path),
    }
    def append_measurement(latest: dict[str, Any]) -> None:
        if latest.get("config_sha256") != config_sha256:
            raise ValueError("run manifest configuration hash changed during measurement")
        latest.setdefault("measurements", []).append(measurement)

    update_manifest(manifest_path, append_measurement)
    print(json.dumps(measurement, ensure_ascii=False, indent=2))
    return exit_code if integrity_ok else 2


def attach_score(args: argparse.Namespace, result: dict[str, Any]) -> None:
    kind = args.command.removeprefix("score-")
    stage = {"transcript": "transcription", "diarization": "diarization", "minutes": "minutes"}[kind]
    reference_key = {"transcript": "transcript_path", "diarization": "diarization_path", "minutes": "minutes_truth_path"}[kind]
    run_dir = Path(args.run).resolve()
    manifest_path = run_dir / "run.json"
    manifest = load_json(manifest_path)
    if manifest.get("config_sha256") != sha256_file(run_dir / "config.toml"):
        raise ValueError("run config snapshot has changed")
    measurements = [item for item in manifest.get("measurements", [])
                    if item.get("trial_id") == args.trial_id and item.get("stage") == stage
                    and item.get("exit_code") == 0 and item.get("integrity_ok") is True]
    if len(measurements) != 1:
        raise ValueError(f"trial requires exactly one successful {stage} measurement")
    measurement = measurements[0]
    dataset_id = measurement["dataset_id"]
    evidence = manifest.get("dataset_evidence", {}).get(dataset_id, {}).get(reference_key)
    reference_path = Path(args.reference).resolve()
    hypothesis_path = Path(args.hypothesis).resolve()
    reference_hash = sha256_file(reference_path)
    hypothesis_hash = sha256_file(hypothesis_path)
    if not isinstance(evidence, dict) or reference_hash != evidence.get("sha256"):
        raise ValueError("reference does not match the frozen dataset truth")
    output_hashes = {item.get("sha256") for item in measurement.get("output_files", [])}
    if kind == "transcript":
        for derivation in manifest.get("derivations", []):
            if (derivation.get("trial_id") == args.trial_id
                    and derivation.get("kind") == "whisper-json-to-transcript"
                    and derivation.get("input_sha256") in output_hashes
                    and sha256_if_available(Path(derivation.get("input_path", ""))) == derivation.get("input_sha256")):
                output_hashes.add(derivation.get("output_sha256"))
    if hypothesis_hash not in output_hashes:
        raise ValueError("hypothesis does not match a benchmark output or verified conversion")
    if kind == "minutes" and (not args.matches or not args.reviewer):
        raise ValueError("attached minutes scores require --matches and --reviewer")
    terms_path = Path(args.terms).resolve() if getattr(args, "terms", None) else None
    frozen_terms = manifest.get("dataset_evidence", {}).get(dataset_id, {}).get("terms_path")
    if kind == "transcript" and frozen_terms and (
        not isinstance(frozen_terms, dict) or terms_path is None or sha256_file(terms_path) != frozen_terms.get("sha256")
    ):
        raise ValueError("transcript terms do not match the frozen dataset lexicon")
    if kind == "transcript" and terms_path is not None and not frozen_terms:
        raise ValueError("transcript terms have no frozen dataset lexicon")
    score_entry = {
        "kind": kind,
        "trial_id": args.trial_id,
        "dataset_id": dataset_id,
        "reference_path": str(reference_path),
        "reference_sha256": reference_hash,
        "hypothesis_path": str(hypothesis_path),
        "hypothesis_sha256": hypothesis_hash,
        "matches_path": str(Path(args.matches).resolve()) if getattr(args, "matches", None) else None,
        "matches_sha256": sha256_file(Path(args.matches)) if getattr(args, "matches", None) else None,
        "terms_path": str(terms_path) if terms_path else None,
        "terms_sha256": sha256_file(terms_path) if terms_path else None,
        "reviewer": args.reviewer,
        "created_at_utc_ms": int(time.time() * 1000),
        "result": result,
    }

    def append_score(latest: dict[str, Any]) -> None:
        if latest.get("config_sha256") != manifest.get("config_sha256"):
            raise ValueError("run config snapshot has changed")
        if sum(item.get("trial_id") == args.trial_id and item.get("stage") == stage
               and item.get("exit_code") == 0 and item.get("integrity_ok") is True
               for item in latest.get("measurements", [])) != 1:
            raise ValueError(f"trial requires exactly one successful {stage} measurement")
        if any(item.get("trial_id") == args.trial_id and item.get("kind") == kind
               for item in latest.get("scores", [])):
            raise ValueError("this trial already has a score of the selected kind")
        latest.setdefault("scores", []).append(score_entry)

    update_manifest(manifest_path, append_score)


def command_score(args: argparse.Namespace, scorer: Any) -> int:
    if bool(args.run) != bool(args.trial_id):
        raise ValueError("--run and --trial-id must be supplied together")
    reference = load_json(Path(args.reference))
    hypothesis = load_json(Path(args.hypothesis))
    if getattr(args, "matches", None):
        result = scorer(reference, hypothesis, load_json(Path(args.matches)))
    elif getattr(args, "terms", None):
        result = scorer(reference, hypothesis, load_json(Path(args.terms)))
    else:
        result = scorer(reference, hypothesis)
    output = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        with Path(args.output).open("x", encoding="utf-8") as handle:
            handle.write(output)
    if args.run:
        attach_score(args, result)
    if not args.output:
        print(output, end="")
    return 0


def command_convert_whisper(args: argparse.Namespace) -> int:
    input_path = Path(args.input).resolve()
    if bool(args.run) != bool(args.trial_id):
        raise ValueError("--run and --trial-id must be supplied together")
    manifest_path: Path | None = None
    manifest: dict[str, Any] | None = None
    if args.run:
        if not args.output:
            raise ValueError("--output is required when attaching a conversion to a run")
        manifest_path = Path(args.run).resolve() / "run.json"
        manifest = load_json(manifest_path)
        measurements = [item for item in manifest.get("measurements", [])
                        if item.get("trial_id") == args.trial_id and item.get("stage") == "transcription"
                        and item.get("exit_code") == 0 and item.get("integrity_ok") is True]
        if len(measurements) != 1 or sha256_file(input_path) not in {
            entry.get("sha256") for entry in measurements[0].get("output_files", [])
        }:
            raise ValueError("whisper JSON must match a successful transcription output in the trial")
        if Path(args.output).resolve() == input_path:
            raise ValueError("converted output must not overwrite the raw whisper JSON")
        if Path(args.output).exists():
            raise ValueError("converted output must name a new file")
    result = convert_whisper_json(load_json(input_path))
    output = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        with Path(args.output).open("x", encoding="utf-8") as handle:
            handle.write(output)
    else:
        print(output, end="")
    if manifest is not None and manifest_path is not None:
        output_path = Path(args.output).resolve()
        derivation = {
            "kind": "whisper-json-to-transcript",
            "trial_id": args.trial_id,
            "input_path": str(input_path),
            "input_sha256": sha256_file(input_path),
            "output_path": str(output_path),
            "output_sha256": sha256_file(output_path),
        }

        def append_derivation(latest: dict[str, Any]) -> None:
            if latest.get("config_sha256") != manifest.get("config_sha256"):
                raise ValueError("run config snapshot has changed")
            latest.setdefault("derivations", []).append(derivation)

        update_manifest(manifest_path, append_derivation)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.set_defaults(handler=lambda _: parser.print_help() or 0)
    subparsers = parser.add_subparsers(dest="command")
    doctor = subparsers.add_parser("doctor", help="print the local evaluation environment")
    doctor.set_defaults(handler=command_doctor)
    validate = subparsers.add_parser("validate", help="validate devices, datasets, and scenarios")
    validate.add_argument("--config", default=str(DEFAULT_CONFIG))
    validate.add_argument("--strict", action="store_true", help="treat unresolved prerequisites as failure")
    validate.set_defaults(handler=command_validate)
    new_run = subparsers.add_parser("new-run", help="create a run with a frozen config snapshot")
    new_run.add_argument("--config", default=str(DEFAULT_CONFIG))
    new_run.add_argument("--name", required=True)
    new_run.set_defaults(handler=command_new_run)
    list_scenarios = subparsers.add_parser("list-scenarios", help="expand the required recording matrix")
    list_scenarios.add_argument("--config", default=str(DEFAULT_CONFIG))
    list_scenarios.add_argument("--format", choices=("csv", "json"), default="csv")
    list_scenarios.set_defaults(handler=command_list_scenarios)
    benchmark = subparsers.add_parser("benchmark", help="run a local engine and append reproducible performance data")
    benchmark.add_argument("--run", required=True)
    benchmark.add_argument("--stage", choices=("audio-preparation", "transcription", "diarization", "integration", "minutes"), required=True)
    benchmark.add_argument("--engine", required=True)
    benchmark.add_argument("--profile", choices=("standard", "high-accuracy"), required=True)
    benchmark.add_argument("--run-kind", choices=("cold", "warm"), default="cold")
    benchmark.add_argument("--variant", choices=("baseline", "terminology"), required=True)
    benchmark.add_argument("--trial-id", required=True, help="same ID for every stage of one end-to-end attempt")
    benchmark.add_argument("--device-id", required=True)
    benchmark.add_argument("--dataset-id", required=True)
    benchmark.add_argument("--model-file", action="append", default=[], help="repeat for every model file used by this stage")
    benchmark.add_argument("--engine-settings-file", help="stable engine settings JSON, required for target-device comparisons")
    benchmark.add_argument("--dependency-file", action="append", default=[], help="repeat for additional stage inputs to hash and verify")
    benchmark.add_argument("--output-file", action="append", default=[], help="repeat for result files to hash after the command")
    benchmark.add_argument("--input", required=True)
    benchmark.add_argument("--input-duration", type=float)
    benchmark.add_argument("--cwd")
    benchmark.add_argument("executable", nargs=argparse.REMAINDER)
    benchmark.set_defaults(handler=command_benchmark)
    whisper = subparsers.add_parser("convert-whisper", help="convert whisper.cpp segment JSON to MON evaluation transcript")
    whisper.add_argument("--input", required=True)
    whisper.add_argument("--output")
    whisper.add_argument("--run", help="attach provenance to an evaluation run")
    whisper.add_argument("--trial-id")
    whisper.set_defaults(handler=command_convert_whisper)
    for name, scorer in (("score-transcript", score_transcript), ("score-diarization", score_diarization), ("score-minutes", score_minutes)):
        score = subparsers.add_parser(name)
        score.add_argument("--reference", required=True)
        score.add_argument("--hypothesis", required=True)
        score.add_argument("--run", help="attach a score to a measured trial")
        score.add_argument("--trial-id")
        score.add_argument("--reviewer", help="required for attached minutes scores")
        if name == "score-minutes":
            score.add_argument("--matches", help="reviewer-provided one-to-one semantic item matches")
        if name == "score-transcript":
            score.add_argument("--terms", help="frozen terminology list for occurrence-based accuracy comparison")
        score.add_argument("--output")
        score.set_defaults(handler=lambda args, selected=scorer: command_score(args, selected))
    return parser


def main(argv: list[str] | None = None) -> int:
    try:
        args = build_parser().parse_args(argv)
        return args.handler(args)
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
