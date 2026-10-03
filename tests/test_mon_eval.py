import importlib.util
import io
import json
import sys
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch


MODULE_PATH = Path(__file__).resolve().parents[1] / "tools" / "mon_eval.py"
SPEC = importlib.util.spec_from_file_location("mon_eval", MODULE_PATH)
mon_eval = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
sys.modules[SPEC.name] = mon_eval
SPEC.loader.exec_module(mon_eval)


class HostRequirementTests(unittest.TestCase):
    def test_required_os_minima_reject_old_unknown_and_server_hosts(self):
        self.assertTrue(mon_eval.host_meets_required_os({"platform": "macos", "os_major": 15}))
        self.assertFalse(mon_eval.host_meets_required_os({"platform": "macos", "os_major": 14}))
        self.assertFalse(mon_eval.host_meets_required_os({"platform": "macos", "os_major": True}))
        self.assertTrue(mon_eval.host_meets_required_os(
            {"platform": "windows", "os_build": 26100, "os_product_type": 1}))
        self.assertFalse(mon_eval.host_meets_required_os(
            {"platform": "windows", "os_build": 26000, "os_product_type": 1}))
        self.assertFalse(mon_eval.host_meets_required_os(
            {"platform": "windows", "os_build": 26100, "os_product_type": 3}))


class TextScoringTests(unittest.TestCase):
    def test_edit_distance(self):
        self.assertEqual(mon_eval.edit_distance("kitten", "sitting"), 3)

    def test_cer_normalizes_japanese_punctuation_and_width(self):
        score = mon_eval.score_transcript({"text": "ＡＩ、会議。"}, {"text": "ai会議"})
        self.assertEqual(score["cer"], 0)

    def test_wer_reports_none_for_japanese_without_spaces(self):
        score = mon_eval.score_transcript({"text": "会議です"}, {"text": "会議でした"})
        self.assertIsNone(score["reference_words"])
        self.assertIsNone(score["wer"])

    def test_utterance_boundaries_do_not_fabricate_wer_tokens(self):
        reference = {"utterances": [{"text": "会議"}, {"text": "です"}]}
        hypothesis = {"utterances": [{"text": "会議"}, {"text": "でした"}]}
        score = mon_eval.score_transcript(reference, hypothesis)
        self.assertEqual(score["cer"], 2 / 4)
        self.assertIsNone(score["reference_words"])
        self.assertIsNone(score["wer"])

    def test_explicit_word_boundaries_still_enable_wer(self):
        score = mon_eval.score_transcript({"text": "hello world"}, {"text": "hello there"})
        self.assertEqual(score["wer"], 0.5)

    def test_wer_requires_hypothesis_word_boundaries_too(self):
        score = mon_eval.score_transcript({"text": "hello world"}, {"utterances": [{"text": "hello"}, {"text": "world"}]})
        self.assertEqual(score["cer"], 0)
        self.assertIsNone(score["wer"])

    def test_terminology_counts_detect_misses_and_extra_occurrences(self):
        terms = {"schema_version": 1, "terms": ["OpenAI", "AI", "東京"]}
        score = mon_eval.score_transcript(
            {"text": "OpenAI と東京"}, {"text": "OpenAI と AI と AI"}, terms,
        )
        self.assertEqual(score["term_reference_occurrences"], 2)
        self.assertEqual(score["term_hypothesis_occurrences"], 3)
        self.assertEqual(score["term_recall"], 0.5)
        self.assertEqual(score["term_precision"], 1 / 3)
        self.assertEqual(score["term_details"][1]["reference_occurrences"], 0)

    def test_terminology_rejects_duplicate_normalized_terms(self):
        with self.assertRaisesRegex(ValueError, "duplicated after normalization"):
            mon_eval.score_transcript({"text": "AI"}, {"text": "AI"}, {"schema_version": 1, "terms": ["ＡＩ", "ai"]})

    def test_terminology_keeps_boundaries_between_repeated_english_words(self):
        score = mon_eval.score_transcript(
            {"text": "AI AI"}, {"text": "AI AI"}, {"schema_version": 1, "terms": ["AI"]},
        )
        self.assertEqual(score["term_reference_occurrences"], 2)
        self.assertEqual(score["term_hypothesis_occurrences"], 2)
        self.assertEqual(score["term_recall"], 1)

    def test_terminology_requires_explicit_text_not_utterance_joining(self):
        with self.assertRaisesRegex(ValueError, "requires explicit text"):
            mon_eval.score_transcript(
                {"utterances": [{"text": "AI"}, {"text": "AI"}]},
                {"text": "AI AI"}, {"schema_version": 1, "terms": ["AI"]},
            )

    def test_whisper_segment_offsets_are_milliseconds(self):
        converted = mon_eval.convert_whisper_json({"transcription": [{"offsets": {"from": 120, "to": 430}, "text": " こんにちは"}]})
        self.assertEqual(converted["utterances"][0]["start_us"], 120000)
        self.assertEqual(converted["utterances"][0]["end_us"], 430000)

    def test_boolean_whisper_timestamp_is_not_a_millisecond(self):
        with self.assertRaisesRegex(ValueError, "invalid millisecond offsets"):
            mon_eval.convert_whisper_json({"transcription": [{"offsets": {"from": False, "to": 10}, "text": "hello"}]})


class DiarizationScoringTests(unittest.TestCase):
    def test_label_permutation_is_not_an_error(self):
        reference = {"segments": [{"start_us": 0, "end_us": 10, "speaker": "alice"}, {"start_us": 10, "end_us": 20, "speaker": "bob"}]}
        hypothesis = {"segments": [{"start_us": 0, "end_us": 10, "speaker": "x"}, {"start_us": 10, "end_us": 20, "speaker": "y"}]}
        score = mon_eval.score_diarization(reference, hypothesis)
        self.assertEqual(score["speaker_error_rate"], 0)

    def test_miss_and_false_alarm_are_time_weighted(self):
        reference = {"segments": [{"start_us": 0, "end_us": 10, "speaker": "a"}]}
        hypothesis = {"segments": [{"start_us": 5, "end_us": 15, "speaker": "x"}]}
        score = mon_eval.score_diarization(reference, hypothesis)
        self.assertEqual(score["miss_us"], 5)
        self.assertEqual(score["false_alarm_us"], 5)
        self.assertEqual(score["speaker_error_rate"], 1)

    def test_overlap_is_scored_per_speaker(self):
        reference = {"segments": [{"start_us": 0, "end_us": 10, "speaker": "a"}, {"start_us": 5, "end_us": 10, "speaker": "b"}]}
        hypothesis = {"segments": [{"start_us": 0, "end_us": 10, "speaker": "x"}]}
        score = mon_eval.score_diarization(reference, hypothesis)
        self.assertEqual(score["miss_us"], 5)
        self.assertEqual(score["reference_speaker_time_us"], 15)
        self.assertEqual(score["overlap_duration_us"], 5)
        self.assertEqual(score["overlap_recall"], 0)
        self.assertIsNone(score["overlap_precision"])

    def test_overlap_detection_counts_true_and_false_positive_time(self):
        reference = {"segments": [{"start_us": 0, "end_us": 20, "speaker": "a"}, {"start_us": 5, "end_us": 15, "speaker": "b"}]}
        hypothesis = {"segments": [{"start_us": 0, "end_us": 20, "speaker": "x"}, {"start_us": 10, "end_us": 20, "speaker": "y"}]}
        score = mon_eval.score_diarization(reference, hypothesis)
        self.assertEqual(score["overlap_duration_us"], 10)
        self.assertEqual(score["predicted_overlap_duration_us"], 10)
        self.assertEqual(score["correctly_detected_overlap_duration_us"], 5)
        self.assertEqual(score["overlap_precision"], 0.5)
        self.assertEqual(score["overlap_recall"], 0.5)
        self.assertEqual(score["boundary_tolerance_us"], 0)

    def test_overlap_recall_is_undefined_without_reference_overlap(self):
        reference = {"segments": [{"start_us": 0, "end_us": 10, "speaker": "a"}]}
        hypothesis = {"segments": [{"start_us": 0, "end_us": 10, "speaker": "x"}, {"start_us": 0, "end_us": 10, "speaker": "y"}]}
        score = mon_eval.score_diarization(reference, hypothesis)
        self.assertIsNone(score["overlap_recall"])
        self.assertEqual(score["overlap_precision"], 0)

    def test_ten_speaker_mapping_is_exact(self):
        reference = {"segments": [{"start_us": i * 10, "end_us": (i + 1) * 10, "speaker": f"s{i}"} for i in range(10)]}
        hypothesis = {"segments": [{"start_us": i * 10, "end_us": (i + 1) * 10, "speaker": f"h{9 - i}"} for i in range(10)]}
        score = mon_eval.score_diarization(reference, hypothesis)
        self.assertEqual(score["speaker_error_rate"], 0)
        self.assertEqual(len(score["speaker_mapping"]), 10)

    def test_boolean_segment_timestamp_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "requires 0 <= start_us"):
            mon_eval.score_diarization({"segments": [{"start_us": False, "end_us": 10, "speaker": "a"}]}, {"segments": []})


class MinutesScoringTests(unittest.TestCase):
    def test_minutes_flags_unsupported_items_and_bad_evidence(self):
        reference = {"items": [{"id": "d1", "kind": "decision", "body": "決定", "assignee": None, "due": None, "evidence_utterance_ids": ["u1"]}]}
        hypothesis = {"items": [{"id": "d1", "kind": "decision", "body": "決定", "assignee": None, "due": None, "evidence_utterance_ids": []}, {"id": "x", "kind": "todo", "body": "追加", "assignee": None, "due": None, "evidence_utterance_ids": []}]}
        score = mon_eval.score_minutes(reference, hypothesis)
        self.assertEqual(score["precision"], 0.5)
        self.assertEqual(score["recall"], 1)
        self.assertEqual(score["evidence_recall"], 0)
        self.assertIsNone(score["evidence_precision"])
        self.assertEqual(score["unsupported_ids"], ["x"])

    def test_minutes_evidence_precision_counts_extra_ids(self):
        reference = {"items": [{"id": "d1", "kind": "decision", "body": "決定", "evidence_utterance_ids": ["u1", "u2"]}]}
        hypothesis = {"items": [{"id": "d1", "kind": "decision", "body": "決定", "evidence_utterance_ids": ["u1", "u3"]}]}
        score = mon_eval.score_minutes(reference, hypothesis)
        self.assertEqual(score["evidence_precision"], 0.5)
        self.assertEqual(score["evidence_recall"], 0.5)
        self.assertEqual(score["mismatches"][0]["missing_evidence"], ["u2"])
        self.assertEqual(score["mismatches"][0]["extra_evidence"], ["u3"])

    def test_reviewer_can_match_items_with_different_ids(self):
        reference = {"items": [{"id": "decision-1", "kind": "decision", "body": "決定", "assignee": None, "due": None, "evidence_utterance_ids": ["u1"]}]}
        hypothesis = {"items": [{"id": "generated-7", "kind": "decision", "body": "同じ決定", "assignee": None, "due": None, "evidence_utterance_ids": ["u1"]}]}
        matches = {"schema_version": 1, "matches": [{"reference_id": "decision-1", "hypothesis_id": "generated-7"}]}
        score = mon_eval.score_minutes(reference, hypothesis, matches)
        self.assertEqual(score["precision"], 1)
        self.assertEqual(score["recall"], 1)
        self.assertEqual(score["matched_pairs"], matches["matches"])

    def test_misclassified_decision_is_not_an_extraction_true_positive(self):
        reference = {"items": [{"id": "d1", "kind": "decision", "body": "採用を決定", "evidence_utterance_ids": ["u1"]}]}
        hypothesis = {"items": [{"id": "t1", "kind": "todo", "body": "採用を決定", "evidence_utterance_ids": ["u1"]}]}
        matches = {"schema_version": 1, "matches": [{"reference_id": "d1", "hypothesis_id": "t1"}]}
        score = mon_eval.score_minutes(reference, hypothesis, matches)
        self.assertEqual(score["precision"], 0)
        self.assertEqual(score["recall"], 0)
        self.assertEqual(score["kind_scores"]["decision"]["recall"], 0)
        self.assertEqual(score["kind_scores"]["todo"]["precision"], 0)
        self.assertEqual(score["missing_ids"], ["d1"])
        self.assertEqual(score["unsupported_ids"], ["t1"])
        self.assertEqual(score["misclassified_pairs"][0]["reference_kind"], "decision")

    def test_todo_assignee_and_due_are_scored_separately(self):
        reference = {"items": [{"id": "t1", "kind": "todo", "body": "確認", "assignee": "A", "due": "2026-10-10"}]}
        hypothesis = {"items": [{"id": "t1", "kind": "todo", "body": "確認", "assignee": "B", "due": "2026-10-10"}]}
        score = mon_eval.score_minutes(reference, hypothesis)
        self.assertEqual(score["todo_assignee_accuracy"], 0)
        self.assertEqual(score["todo_due_accuracy"], 1)

    def test_duplicate_evidence_ids_are_invalid(self):
        minutes = {"items": [{"id": "d1", "kind": "decision", "body": "決定", "evidence_utterance_ids": ["u1", "u1"]}]}
        with self.assertRaisesRegex(ValueError, "contains duplicates"):
            mon_eval.score_minutes(minutes, minutes)


class ConfigTests(unittest.TestCase):
    def test_repository_config_has_no_structural_errors(self):
        config = mon_eval.load_config(mon_eval.DEFAULT_CONFIG)
        errors, warnings = mon_eval.validate_config(config, mon_eval.DEFAULT_CONFIG)
        self.assertEqual(errors, [])
        self.assertGreaterEqual(len(warnings), 1)

    def test_recording_matrix_has_all_32_combinations(self):
        config = mon_eval.load_config(mon_eval.DEFAULT_CONFIG)
        scenarios = mon_eval.expand_recording_matrix(config)
        self.assertEqual(len(scenarios), 32)
        self.assertEqual(len({scenario["id"] for scenario in scenarios}), 32)

    def test_available_dataset_requires_consent(self):
        config = mon_eval.load_config(mon_eval.DEFAULT_CONFIG)
        config["datasets"][0]["status"] = "available"
        errors, _ = mon_eval.validate_config(config, mon_eval.DEFAULT_CONFIG)
        self.assertTrue(any("consent_confirmed" in error for error in errors))

    def test_required_dataset_must_declare_overlap_and_english_terms(self):
        config = mon_eval.load_config(mon_eval.DEFAULT_CONFIG)
        config["datasets"][0]["contains_overlap"] = False
        config["datasets"][0]["contains_english_terms"] = False
        errors, _ = mon_eval.validate_config(config, mon_eval.DEFAULT_CONFIG)
        self.assertTrue(any("contains_overlap must be true" in error for error in errors))
        self.assertTrue(any("contains_english_terms must be true" in error for error in errors))

    def test_required_dataset_must_have_ten_speakers(self):
        config = mon_eval.load_config(mon_eval.DEFAULT_CONFIG)
        config["datasets"][0]["speaker_count"] = 9
        errors, _ = mon_eval.validate_config(config, mon_eval.DEFAULT_CONFIG)
        self.assertTrue(any("speaker_count must be 10" in error for error in errors))

    def test_required_device_must_fix_power_source(self):
        config = mon_eval.load_config(mon_eval.DEFAULT_CONFIG)
        config["devices"][1]["power_source"] = "TBD"
        errors, _ = mon_eval.validate_config(config, mon_eval.DEFAULT_CONFIG)
        self.assertTrue(any("power_source must be AC or battery" in error for error in errors))

    def test_performance_matrix_requires_16_gib_on_both_platforms(self):
        config = mon_eval.load_config(mon_eval.DEFAULT_CONFIG)
        config["devices"] = [item for item in config["devices"] if item["id"] != "macos-arm64-16gb"]
        errors, _ = mon_eval.validate_config(config, mon_eval.DEFAULT_CONFIG)
        self.assertTrue(any("16 GiB macos" in error for error in errors))

    def test_dataset_rejects_broken_evidence_reference(self):
        transcript = {"schema_version": 1, "text": "決定", "utterances": [{"id": "u1", "start_us": 0, "end_us": 100, "speaker": "s1", "text": "決定"}]}
        diarization = {"schema_version": 1, "segments": [{"start_us": 0, "end_us": 100, "speaker": "s1"}]}
        minutes = {"schema_version": 1, "items": [{"id": "d1", "kind": "decision", "body": "決定", "assignee": None, "due": None, "evidence_utterance_ids": ["missing"]}]}
        errors = mon_eval.validate_dataset_documents(transcript, diarization, minutes, 1, 1)
        self.assertTrue(any("unknown utterance missing" in error for error in errors))

    def test_dataset_accepts_consistent_documents(self):
        transcript = {"schema_version": 1, "text": "決定", "utterances": [{"id": "u1", "start_us": 0, "end_us": 100, "speaker": "s1", "text": "決定"}]}
        diarization = {"schema_version": 1, "segments": [{"start_us": 0, "end_us": 100, "speaker": "s1"}]}
        minutes = {"schema_version": 1, "items": [{"id": "d1", "kind": "decision", "body": "決定", "assignee": None, "due": None, "evidence_utterance_ids": ["u1"]}]}
        self.assertEqual(mon_eval.validate_dataset_documents(transcript, diarization, minutes, 1, 1), [])

    def test_dataset_rejects_declared_overlap_without_distinct_simultaneous_speakers(self):
        transcript = {"schema_version": 1, "utterances": [{"id": "u1", "start_us": 0, "end_us": 10, "speaker": "s1", "text": "はい"}]}
        diarization = {"schema_version": 1, "segments": [
            {"start_us": 0, "end_us": 10, "speaker": "s1"},
            {"start_us": 10, "end_us": 20, "speaker": "s2"},
        ]}
        minutes = {"schema_version": 1, "items": []}
        errors = mon_eval.validate_dataset_documents(transcript, diarization, minutes, 1, 2, require_overlap=True)
        self.assertTrue(any("no simultaneous speech" in error for error in errors))
        diarization["segments"][1]["start_us"] = 5
        self.assertEqual(mon_eval.validate_dataset_documents(transcript, diarization, minutes, 1, 2, require_overlap=True), [])

    def test_dataset_rejects_utterance_at_wrong_speaker_time(self):
        transcript = {"schema_version": 1, "utterances": [{"id": "u1", "start_us": 0, "end_us": 10, "speaker": "s1", "text": "はい"}]}
        diarization = {"schema_version": 1, "segments": [{"start_us": 20, "end_us": 30, "speaker": "s1"}]}
        minutes = {"schema_version": 1, "items": []}
        errors = mon_eval.validate_dataset_documents(transcript, diarization, minutes, 1, 1)
        self.assertTrue(any("no overlapping diarization segment" in error for error in errors))

    def test_required_minutes_truth_covers_decision_confirmed_and_unknown_todos(self):
        transcript = {"schema_version": 1, "utterances": [{"id": "u1", "start_us": 0, "end_us": 10,
                                                               "speaker": "s1", "text": "確認"}]}
        diarization = {"schema_version": 1, "segments": [{"start_us": 0, "end_us": 10, "speaker": "s1"}]}
        minutes = {"schema_version": 1, "items": [{"id": "d1", "kind": "decision", "body": "決定",
                                                      "evidence_utterance_ids": ["u1"]}]}
        errors = mon_eval.validate_dataset_documents(
            transcript, diarization, minutes, 1, 1, require_minutes_coverage=True,
        )
        self.assertTrue(any("explicit assignee and due" in error for error in errors))
        self.assertTrue(any("unconfirmed assignee or due" in error for error in errors))
        minutes["items"].extend([
            {"id": "t1", "kind": "todo", "body": "担当作業", "assignee": "A", "due": "2026-10-10",
             "evidence_utterance_ids": ["u1"]},
            {"id": "t2", "kind": "todo", "body": "未確認作業", "assignee": None, "due": None,
             "evidence_utterance_ids": ["u1"]},
        ])
        self.assertEqual(mon_eval.validate_dataset_documents(
            transcript, diarization, minutes, 1, 1, require_minutes_coverage=True,
        ), [])


class BenchmarkTests(unittest.TestCase):
    def test_manifest_updates_keep_concurrent_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.json"
            path.write_text('{"measurements": []}', encoding="utf-8")

            def append(index):
                mon_eval.update_manifest(path, lambda manifest: manifest["measurements"].append(index))

            with ThreadPoolExecutor(max_workers=4) as executor:
                list(executor.map(append, range(12)))
            self.assertEqual(sorted(json.loads(path.read_text(encoding="utf-8"))["measurements"]), list(range(12)))

    def test_terminology_trial_requires_frozen_terms_dependency(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "run"
            run.mkdir()
            source = root / "audio.bin"
            source.write_bytes(b"audio")
            model = root / "model.bin"
            model.write_bytes(b"model")
            settings = root / "engine-settings.json"
            settings.write_text(json.dumps({"schema_version": 1, "engine": "fixture",
                                            "settings": {"threads": 2}}), encoding="utf-8")
            terms = root / "terms.json"
            terms.write_text(json.dumps({"schema_version": 1, "terms": ["MON"]}), encoding="utf-8")
            config = run / "config.toml"
            config.write_text(
                '[[devices]]\nid = "mac-16"\nstatus = "available"\nrequired = true\nplatform = "macos"\n'
                'architecture = "arm64"\nmemory_gib = 16\ncpu = "Fixture CPU"\npower_source = "AC"\n'
                '[[datasets]]\nid = "fixture"\nstatus = "available"\nconsent_confirmed = true\n'
                f'duration_seconds = 10\nsha256 = "{mon_eval.sha256_file(source)}"\nterms_path = "{terms}"\n', encoding="utf-8",
            )
            (run / "run.json").write_text(json.dumps({
                "config_sha256": mon_eval.sha256_file(config),
                "dataset_evidence": {"fixture": {"terms_path": {"sha256": mon_eval.sha256_file(terms)}}},
                "measurements": [],
            }), encoding="utf-8")
            args = type("Args", (), {
                "run": str(run), "stage": "transcription", "engine": "fixture", "profile": "standard",
                "run_kind": "cold", "variant": "terminology", "trial_id": "trial-1",
                "device_id": "mac-16", "dataset_id": "fixture", "model_file": [str(model)],
                "engine_settings_file": str(settings),
                "dependency_file": [], "output_file": [], "input": str(source), "input_duration": 10.0,
                "cwd": None, "executable": ["--", sys.executable, "-c", "print('ok')"],
            })()
            facts = {"platform": "macos", "architecture": "arm64", "memory_gib_rounded": 16,
                     "physical_memory_bytes": 16 * 1024 ** 3, "os_version": "test",
                     "os_major": 15, "os_build": None, "os_product_type": None,
                     "cpu_model": "Fixture CPU", "power_source": "AC"}
            with patch.object(mon_eval, "host_facts", return_value=facts):
                with self.assertRaisesRegex(ValueError, "must declare the frozen terms"):
                    mon_eval.command_benchmark(args)
                args.dependency_file = [str(terms)]
                with redirect_stdout(io.StringIO()):
                    self.assertEqual(mon_eval.command_benchmark(args), 0)
                args.variant = "baseline"
                args.trial_id = "trial-2"
                with self.assertRaisesRegex(ValueError, "must not declare the frozen terms"):
                    mon_eval.command_benchmark(args)
            args.variant = "terminology"
            args.trial_id = "trial-power-change"
            with patch.object(mon_eval, "host_facts", side_effect=[facts, {**facts, "power_source": "battery"}]):
                with redirect_stdout(io.StringIO()):
                    self.assertEqual(mon_eval.command_benchmark(args), 2)
            manifest = json.loads((run / "run.json").read_text(encoding="utf-8"))
            self.assertFalse(manifest["measurements"][-1]["integrity_ok"])
            self.assertFalse(manifest["measurements"][-1]["required_host_stable"])

    def test_benchmark_records_hash_timing_and_logs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "run"
            run.mkdir()
            input_path = root / "input.txt"
            input_path.write_text("audio-placeholder", encoding="utf-8")
            model_path = root / "model.bin"
            model_path.write_bytes(b"test-model")
            settings_path = root / "engine-settings.json"
            settings_path.write_text(json.dumps({"schema_version": 1, "engine": "fixture",
                                                 "settings": {"threads": 2}}), encoding="utf-8")
            config_path = run / "config.toml"
            config_path.write_text(
                '[[devices]]\nid = "fixture-16gb"\nstatus = "available"\nrequired = true\nplatform = "macos"\n'
                'architecture = "arm64"\nmemory_gib = 16\ncpu = "Fixture CPU"\npower_source = "AC"\n'
                '[[datasets]]\nid = "fixture"\nstatus = "available"\nconsent_confirmed = true\n'
                f'duration_seconds = 10\nsha256 = "{mon_eval.sha256_file(input_path)}"\n',
                encoding="utf-8",
            )
            (run / "run.json").write_text(json.dumps({"config_sha256": mon_eval.sha256_file(config_path), "measurements": []}), encoding="utf-8")
            args = type("Args", (), {
                "run": str(run), "stage": "transcription", "engine": "fixture", "profile": "standard",
                "run_kind": "cold", "variant": "baseline", "trial_id": "trial-1",
                "device_id": "fixture-16gb", "dataset_id": "fixture", "model_file": [str(model_path)],
                "engine_settings_file": str(settings_path),
                "output_file": [],
                "input": str(input_path), "input_duration": 10.0, "cwd": None,
                "executable": ["--", sys.executable, "-c", "print('ok')"],
            })()
            facts = {"platform": "macos", "architecture": "arm64", "memory_gib_rounded": 16,
                     "physical_memory_bytes": 16 * 1024 ** 3, "os_version": "test",
                     "os_major": 15, "os_build": None, "os_product_type": None,
                     "cpu_model": "Fixture CPU", "power_source": "AC"}
            with patch.object(mon_eval, "host_facts", return_value=facts), redirect_stdout(io.StringIO()):
                self.assertEqual(mon_eval.command_benchmark(args), 0)
            manifest = json.loads((run / "run.json").read_text(encoding="utf-8"))
            measurement = manifest["measurements"][0]
            self.assertEqual(measurement["exit_code"], 0)
            self.assertEqual(measurement["input_sha256"], mon_eval.sha256_file(input_path))
            self.assertEqual(measurement["model_files"][0]["sha256"], mon_eval.sha256_file(model_path))
            self.assertEqual(measurement["executable_file"]["sha256"], mon_eval.sha256_file(Path(sys.executable)))
            self.assertEqual(measurement["engine_settings_file"]["sha256"], mon_eval.sha256_file(settings_path))
            self.assertFalse(measurement["executable_file"]["changed_during_run"])
            self.assertEqual(measurement["command"][0], str(Path(sys.executable).resolve()))
            self.assertEqual(measurement["device_id"], "fixture-16gb")
            self.assertEqual(measurement["host_facts"], facts)
            self.assertEqual(measurement["host_facts_at_end"], facts)
            self.assertTrue(measurement["required_host_stable"])
            self.assertEqual(measurement["trial_id"], "trial-1")
            self.assertTrue((run / measurement["stdout_log"]).is_file())
            args.engine_settings_file = None
            with patch.object(mon_eval, "host_facts", return_value=facts):
                with self.assertRaisesRegex(ValueError, "needs --engine-settings-file"):
                    mon_eval.command_benchmark(args)
            args.engine_settings_file = str(settings_path)
            with patch.object(mon_eval, "host_facts", return_value={**facts, "power_source": "battery"}):
                with self.assertRaisesRegex(ValueError, "CPU model or power source"):
                    mon_eval.command_benchmark(args)
            with patch.object(mon_eval, "host_facts", return_value={**facts, "cpu_model": "Different CPU"}):
                with self.assertRaisesRegex(ValueError, "CPU model or power source"):
                    mon_eval.command_benchmark(args)
            with patch.object(mon_eval, "host_facts", return_value={**facts, "os_major": 14}):
                with self.assertRaisesRegex(ValueError, "required device does not meet"):
                    mon_eval.command_benchmark(args)
            args.trial_id = "trial-2"
            args.executable = ["--", sys.executable, "-c", f"from pathlib import Path; Path({str(input_path)!r}).write_text('changed')"]
            with patch.object(mon_eval, "host_facts", return_value=facts), redirect_stdout(io.StringIO()):
                self.assertEqual(mon_eval.command_benchmark(args), 2)
            manifest = json.loads((run / "run.json").read_text(encoding="utf-8"))
            self.assertFalse(manifest["measurements"][1]["integrity_ok"])
            self.assertTrue(manifest["measurements"][1]["input_changed_during_run"])
            input_path.write_text("audio-placeholder", encoding="utf-8")
            dependency = root / "speaker-output.json"
            dependency.write_text("original", encoding="utf-8")
            args.trial_id = "trial-3"
            args.dependency_file = [str(dependency)]
            args.executable = ["--", sys.executable, "-c", f"from pathlib import Path; Path({str(dependency)!r}).write_text('changed')"]
            with patch.object(mon_eval, "host_facts", return_value=facts), redirect_stdout(io.StringIO()):
                self.assertEqual(mon_eval.command_benchmark(args), 2)
            manifest = json.loads((run / "run.json").read_text(encoding="utf-8"))
            third = manifest["measurements"][2]
            self.assertFalse(third["input_changed_during_run"])
            self.assertTrue(third["dependency_files"][0]["changed_during_run"])
            self.assertFalse(third["integrity_ok"])
            manifest["dataset_evidence"] = {"fixture": {"audio_path": {
                "sha256": mon_eval.sha256_file(input_path)}}}
            (run / "run.json").write_text(json.dumps(manifest), encoding="utf-8")
            other_input = root / "other.txt"
            other_input.write_text("different audio", encoding="utf-8")
            args.stage = "audio-preparation"
            args.trial_id = "trial-4"
            args.model_file = []
            args.dependency_file = []
            args.input = str(other_input)
            args.executable = ["--", sys.executable, "-c", "print('ok')"]
            with patch.object(mon_eval, "host_facts", return_value=facts):
                with self.assertRaisesRegex(ValueError, "frozen dataset audio"):
                    mon_eval.command_benchmark(args)
            args.stage = "integration"
            args.trial_id = "spawn-fail"
            args.input = str(input_path)
            unlaunchable = root / "unlaunchable-engine"
            unlaunchable.write_text("not executable", encoding="utf-8")
            args.executable = ["--", str(unlaunchable)]
            with patch.object(mon_eval, "host_facts", return_value=facts):
                with self.assertRaisesRegex(ValueError, "cannot start benchmark command"):
                    mon_eval.command_benchmark(args)
            orphan_log = next(run.glob("spawn-fail-integration-*.stdout.log"))
            args.executable = ["--", sys.executable, "-c", "print('retry')"]
            with patch.object(mon_eval, "host_facts", return_value=facts), redirect_stdout(io.StringIO()):
                self.assertEqual(mon_eval.command_benchmark(args), 0)
            self.assertTrue(orphan_log.is_file())
            self.assertEqual(len(list(run.glob("spawn-fail-integration-*.stdout.log"))), 2)


class ScoreAttachmentTests(unittest.TestCase):
    def test_score_output_never_overwrites_existing_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reference = root / "truth.json"
            hypothesis = root / "hypothesis.json"
            output = root / "score.json"
            reference.write_text('{"text":"hello"}', encoding="utf-8")
            hypothesis.write_text('{"text":"hello"}', encoding="utf-8")
            output.write_text("keep me", encoding="utf-8")
            args = type("Args", (), {"reference": str(reference), "hypothesis": str(hypothesis),
                                     "run": None, "trial_id": None, "matches": None, "terms": None,
                                     "output": str(output)})()
            with self.assertRaises(FileExistsError):
                mon_eval.command_score(args, mon_eval.score_transcript)
            self.assertEqual(output.read_text(encoding="utf-8"), "keep me")

    def test_whisper_conversion_and_score_keep_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "run"
            run.mkdir()
            config = run / "config.toml"
            config.write_text('schema_version = 1\n', encoding="utf-8")
            reference = root / "truth.json"
            reference.write_text(json.dumps({"text": "hello"}), encoding="utf-8")
            raw = root / "whisper.json"
            raw.write_text(json.dumps({"transcription": [{"offsets": {"from": 0, "to": 1000}, "text": "hello"}]}), encoding="utf-8")
            terms = root / "terms.json"
            terms.write_text(json.dumps({"schema_version": 1, "terms": ["hello"]}), encoding="utf-8")
            normalized = root / "hypothesis.json"
            manifest = {
                "config_sha256": mon_eval.sha256_file(config),
                "dataset_evidence": {"sample": {
                    "transcript_path": {"sha256": mon_eval.sha256_file(reference)},
                    "terms_path": {"sha256": mon_eval.sha256_file(terms)},
                }},
                "measurements": [{
                    "trial_id": "trial-1", "stage": "transcription", "dataset_id": "sample",
                    "exit_code": 0, "integrity_ok": True,
                    "output_files": [{"path": str(raw), "sha256": mon_eval.sha256_file(raw)}],
                }],
                "derivations": [], "scores": [],
            }
            (run / "run.json").write_text(json.dumps(manifest), encoding="utf-8")
            convert_args = type("Args", (), {"input": str(raw), "output": str(normalized),
                                               "run": str(run), "trial_id": "trial-1"})()
            self.assertEqual(mon_eval.command_convert_whisper(convert_args), 0)
            with self.assertRaisesRegex(ValueError, "must name a new file"):
                mon_eval.command_convert_whisper(convert_args)
            score_args = type("Args", (), {
                "command": "score-transcript", "run": str(run), "trial_id": "trial-1",
                "reference": str(reference), "hypothesis": str(normalized),
                "matches": None, "terms": str(terms), "reviewer": None, "output": None,
            })()
            with redirect_stdout(io.StringIO()):
                self.assertEqual(mon_eval.command_score(score_args, mon_eval.score_transcript), 0)
            attached = json.loads((run / "run.json").read_text(encoding="utf-8"))
            self.assertEqual(attached["scores"][0]["result"]["cer"], 0)
            self.assertEqual(attached["scores"][0]["result"]["term_recall"], 1)
            self.assertEqual(attached["scores"][0]["terms_sha256"], mon_eval.sha256_file(terms))
            self.assertEqual(attached["scores"][0]["hypothesis_sha256"], mon_eval.sha256_file(normalized))
            raw.write_text("changed", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "hypothesis does not match"):
                mon_eval.attach_score(score_args, {"cer": 0})

    def test_minutes_attachment_requires_explicit_reviewed_matches(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "run"
            run.mkdir()
            config = run / "config.toml"
            config.write_text('schema_version = 1\n', encoding="utf-8")
            minutes = {"schema_version": 1, "items": [{"id": "d1", "kind": "decision", "body": "採用",
                                                       "assignee": None, "due": None,
                                                       "evidence_utterance_ids": ["u1"]}]}
            reference = root / "minutes-truth.json"
            hypothesis = root / "minutes-result.json"
            reference.write_text(json.dumps(minutes), encoding="utf-8")
            hypothesis.write_text(json.dumps(minutes), encoding="utf-8")
            matches = root / "matches.json"
            matches.write_text(json.dumps({"schema_version": 1, "matches": [
                {"reference_id": "d1", "hypothesis_id": "d1"}]}), encoding="utf-8")
            manifest = {
                "config_sha256": mon_eval.sha256_file(config),
                "dataset_evidence": {"sample": {"minutes_truth_path": {"sha256": mon_eval.sha256_file(reference)}}},
                "measurements": [{"trial_id": "trial-1", "stage": "minutes", "dataset_id": "sample",
                                  "exit_code": 0, "integrity_ok": True,
                                  "output_files": [{"path": str(hypothesis), "sha256": mon_eval.sha256_file(hypothesis)}]}],
                "scores": [],
            }
            (run / "run.json").write_text(json.dumps(manifest), encoding="utf-8")
            args = type("Args", (), {
                "command": "score-minutes", "run": str(run), "trial_id": "trial-1",
                "reference": str(reference), "hypothesis": str(hypothesis),
                "matches": str(matches), "reviewer": None, "output": None,
            })()
            with self.assertRaisesRegex(ValueError, "reviewer"):
                mon_eval.command_score(args, mon_eval.score_minutes)
            args.reviewer = "human-1"
            with redirect_stdout(io.StringIO()):
                self.assertEqual(mon_eval.command_score(args, mon_eval.score_minutes), 0)
            attached = json.loads((run / "run.json").read_text(encoding="utf-8"))
            self.assertEqual(attached["scores"][0]["reviewer"], "human-1")
            self.assertEqual(attached["scores"][0]["result"]["evidence_recall"], 1)


if __name__ == "__main__":
    unittest.main()
