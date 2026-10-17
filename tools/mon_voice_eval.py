#!/usr/bin/env python3
"""Read-only calibration of labeled, model-bound voice embeddings.

Input samples carry personId (null for unknown), recordingHash, split
(enrollment/calibration/validation), and vector. Recording hashes must be
exclusive to one split. This evaluates supplied labels; it does not certify
consent, person identities, or that synthetic vectors are real recordings.
The report never changes a product DB or enables automatic identification.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

EXTRACTOR = "sherpa-1.13.8-mono16k-normalized-v1"


def normalize(vector):
    if not isinstance(vector, list) or not 1 <= len(vector) <= 4096:
        raise ValueError("invalid embedding dimension")
    if any(type(v) not in (float, int) or not math.isfinite(v) for v in vector):
        raise ValueError("embedding contains invalid numbers")
    norm = math.hypot(*vector)
    if not math.isfinite(norm) or norm < 1e-12:
        raise ValueError("empty embedding")
    return [v / norm for v in vector]


def hash_value(value):
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def prepare(data):
    if data.get("schemaVersion") != 1 or data.get("extractorVersion") != EXTRACTOR or not hash_value(data.get("modelHash")):
        raise ValueError("unsupported dataset or model identity")
    samples = data.get("samples")
    if not isinstance(samples, list) or not 1 <= len(samples) <= 8192:
        raise ValueError("invalid sample count")
    recordings, groups = {}, {s: [] for s in ("enrollment", "calibration", "validation")}
    dimensions, components = set(), 0
    for sample in samples:
        split = sample.get("split")
        recording = sample.get("recordingHash")
        person = sample.get("personId")
        if split not in groups or not hash_value(recording) or person is not None and (not isinstance(person, str) or not person or len(person) > 256):
            raise ValueError("invalid sample provenance or label")
        if recordings.setdefault(recording, split) != split:
            raise ValueError("a recording was reused between enrollment, calibration and validation")
        vector = normalize(sample.get("vector"))
        components += len(vector)
        dimensions.add(len(vector))
        if len(dimensions) != 1 or components > 8 * 1024 * 1024:
            raise ValueError("mixed dimensions or excessive embeddings")
        groups[split].append((person, vector))
    registered = {}
    for person, vector in groups["enrollment"]:
        if person is None:
            raise ValueError("enrollment must identify a person")
        registered.setdefault(person, []).append(vector)
    if not registered:
        raise ValueError("no enrolled people")
    centroids = {p: normalize([sum(values) / len(vectors) for values in zip(*vectors)]) for p, vectors in registered.items()}
    scored = {}
    for split in ("calibration", "validation"):
        scored[split] = []
        for truth, vector in groups[split]:
            ranking = sorted(((sum(a*b for a, b in zip(vector, centroid)), person) for person, centroid in centroids.items()), key=lambda v: (-v[0], v[1]))
            top, predicted = ranking[0]
            margin = top - (ranking[1][0] if len(ranking) > 1 else -1)
            scored[split].append((truth if truth in centroids else None, predicted, top, margin))
    return scored


def score(cases, threshold, margin):
    accepted = [(truth, prediction) for truth, prediction, similarity, difference in cases if similarity >= threshold and difference >= margin]
    wrong = sum(truth != prediction for truth, prediction in accepted)
    return {"trials": len(cases), "knownTrials": sum(truth is not None for truth, *_ in cases), "unknownTrials": sum(truth is None for truth, *_ in cases), "automaticAssignments": len(accepted), "wrongAssignments": wrong, "wrongAssignmentRate": wrong / len(accepted) if accepted else None}


def acceptable(result):
    return result["knownTrials"] >= 100 and result["unknownTrials"] >= 20 and result["automaticAssignments"] >= 100 and result["wrongAssignmentRate"] <= .01


def evaluate(data):
    groups = prepare(data)
    # Pick on calibration only. Never tune against the independent validation set.
    candidates = []
    for threshold in range(50, 101):
        for margin in range(1, 51):
            result = score(groups["calibration"], threshold / 100, margin / 100)
            if acceptable(result):
                candidates.append((result["automaticAssignments"] - result["wrongAssignments"], -result["wrongAssignments"], threshold, margin))
    digest = hashlib.sha256(json.dumps(data, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode()).hexdigest()
    report = {"schemaVersion": 1, "modelHash": data["modelHash"], "extractorVersion": EXTRACTOR, "datasetHash": digest, "qualityStatus": "insufficient_or_failed", "policy": None}
    if candidates:
        _, _, threshold, margin = max(candidates)
        calibration = score(groups["calibration"], threshold / 100, margin / 100)
        validation = score(groups["validation"], threshold / 100, margin / 100)
        report.update(calibration=calibration, validation=validation)
        if acceptable(validation):
            report.update(qualityStatus="observed_target_met", policy={"threshold": threshold / 100, "margin": margin / 100, "evaluationHash": digest})
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path)
    args = parser.parse_args()
    if args.dataset.stat().st_size > 64 * 1024 * 1024:
        parser.error("dataset exceeds 64 MiB")
    try:
        report = evaluate(json.loads(args.dataset.read_text(encoding="utf-8")))
    except (ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
