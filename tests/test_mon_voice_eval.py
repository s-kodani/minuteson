import copy
import unittest
from tools.mon_voice_eval import EXTRACTOR, evaluate, prepare


def dataset():
    samples = [{"split": "enrollment", "recordingHash": "a"*64, "personId": "person", "vector": [1., 0.]}]
    for split, recording in [("calibration", "b"), ("validation", "c")]:
        samples += [{"split": split, "recordingHash": recording*64, "personId": "person", "vector": [1., 0.]} for _ in range(100)]
        samples += [{"split": split, "recordingHash": recording*64, "personId": None, "vector": [0., 1.]} for _ in range(20)]
    return {"schemaVersion": 1, "modelHash": "d"*64, "extractorVersion": EXTRACTOR, "samples": samples}


class VoiceEvaluationTests(unittest.TestCase):
    def test_independent_known_and_unknown_examples_can_meet_observed_target(self):
        report = evaluate(dataset())
        self.assertEqual(report["qualityStatus"], "observed_target_met")
        self.assertEqual(report["validation"]["wrongAssignments"], 0)
        self.assertEqual(report["validation"]["automaticAssignments"], 100)

    def test_validation_is_not_used_to_retune_thresholds(self):
        data = dataset()
        for sample in data["samples"]:
            if sample["split"] == "validation" and sample["personId"] is None:
                sample["vector"] = [1., 0.]
        self.assertIsNone(evaluate(data)["policy"])

    def test_same_recording_cannot_leak_between_splits(self):
        data = dataset()
        data["samples"][-1]["recordingHash"] = "a"*64
        with self.assertRaisesRegex(ValueError, "reused"):
            prepare(data)

    def test_missing_unknown_cases_and_ambiguous_enrollment_do_not_qualify(self):
        data = dataset()
        data["samples"] = [s for s in data["samples"] if s["personId"] is not None]
        self.assertIsNone(evaluate(data)["policy"])
        data = dataset()
        other = copy.deepcopy(data["samples"][0]); other["personId"] = "other"
        data["samples"].append(other)
        self.assertIsNone(evaluate(data)["policy"])


if __name__ == "__main__":
    unittest.main()
