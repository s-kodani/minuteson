import test from "node:test";
import assert from "node:assert/strict";
import { modelVocabularySupport } from "../src/model-vocabulary-support.ts";

const backends = [
  { id: "whisper.cpp", vocabularyDescription: "表記・読みを文章のヒントとして使用" },
  { id: "registered-a", vocabularyDescription: "表記のみ使用" },
  { id: "registered-b", vocabularyDescription: "別の対応方法" },
];

test("vocabulary support follows the inspected backend rather than the selected or named model", () => {
  const copies = [
    { engine: "asr", backendId: "registered-a", modelName: "same name" },
    { engine: "asr", backendId: "registered-b", modelName: "same name" },
  ];
  assert.equal(modelVocabularySupport(copies[0], backends), "表記のみ使用");
  assert.equal(modelVocabularySupport(copies[1], backends), "別の対応方法");
  assert.equal(modelVocabularySupport(copies[0], [...backends].reverse()), "表記のみ使用");
});

test("legacy whisper resolves its registered description without a backend binding", () => {
  assert.equal(modelVocabularySupport({ engine: "whisper.cpp" }, backends), "表記・読みを文章のヒントとして使用");
});

test("missing backend information remains unknown and unrelated model purposes have no vocabulary field", () => {
  for (const model of [{ engine: "asr" }, { engine: "asr", backendId: "unknown" }, { engine: "whisper.cpp" }]) {
    assert.equal(modelVocabularySupport(model, []), "対応情報を確認できません");
  }
  assert.equal(modelVocabularySupport({ engine: "asr", backendId: "unknown" }, backends), "対応情報を確認できません");
  assert.equal(modelVocabularySupport({ engine: "asr", backendId: "empty" }, [{ id: "empty", vocabularyDescription: " " }]), "対応情報を確認できません");
  assert.equal(modelVocabularySupport({ engine: "sherpa-onnx" }, backends), null);
  assert.equal(modelVocabularySupport({ engine: "llama.cpp" }, backends), null);
});
