import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { canonicalRecipe, downloadByteTotals, downloadDigest, type OfficialModelOption } from "../src/model-download-state.ts";
import { modelChoices, matchesOfficialModel, isAsrEngine } from "../src/model-selection-state.ts";
import { assertProfileView } from "../src/model-profile-state.ts";

const definitions = JSON.parse(readFileSync(new URL("../model-catalog/asr-transducer.json", import.meta.url), "utf8"));
const options: OfficialModelOption[] = await Promise.all(definitions.map(async (definition: any) => {
  const recipe = canonicalRecipe(definition.recipe);
  return { ...definition, definition: JSON.stringify(recipe), tokenLimits: null, review: {
    ...recipe, recipeSchemaVersion: 4, recipeHash: await downloadDigest(recipe), ...downloadByteTotals(recipe),
    totalBytes: recipe.manifest.files.reduce((n, f) => n + f.expectedSize, 0), publisherAuthenticity: "not_verified", installed: false, ready: false,
  } };
}));
test("ASR selection groups registered backends without a model-specific UI branch", () => {
  const whisper = structuredClone(options[0]);
  whisper.id = "whisper-test"; whisper.label = "Whisper"; whisper.review.manifest.engine = "whisper.cpp";
  const other = structuredClone(options[0]);
  other.id = "third-adapter"; other.label = "Another ASR"; other.review.manifest.backend!.id = "test-adapter";
  const choices = modelChoices("asr", [], [whisper, ...options, other], []);
  assert.deepEqual(choices.map(c => c.label), ["Whisper", "ReazonSpeech-k2-v2（軽量）", "ReazonSpeech-k2-v2（FP32）", "Another ASR"]);
  assert.ok(isAsrEngine("whisper.cpp") && isAsrEngine("asr"));
  assert.equal(isAsrEngine("sherpa-onnx"), false);
});
test("both transducer variants share decoder, tokens and VAD and keep distinct encoders", () => {
  const files = options.map(option => new Map(option.review.manifest.files.map(f => [f.relativePath, f.expectedHash])));
  for (const path of ["models/decoder.onnx", "models/tokens.txt", "models/silero_vad.onnx"]) {
    assert.equal(files[0].get(path), files[1].get(path));
  }
  assert.notEqual(files[0].get("models/encoder.int8.onnx"), files[1].get("models/encoder.onnx"));
  assert.deepEqual(Object.keys(options[0].review.manifest.backend!.files), ["decoder", "joiner", "library", "tokens"]);
});
test("a changed transport recipe preserves the installed transducer model identity", () => {
  const option = options[0], m = option.review.manifest;
  const entry = { packageId: "installed", engine: "asr", backendId: m.backend!.id, engineVersion: m.engineVersion, modelName: m.modelName, modelVersion: m.modelVersion,
    totalBytes: option.review.totalBytes, placementRecorded: true, ready: false as const, reviewHash: "a".repeat(64), sourceType: "download" as const,
    download: { operationId: "33333333-3333-4333-8333-333333333333", recipeHash: "b".repeat(64) },
    modelFiles: m.files.filter(f => ["asr", "asr_asset", "vad"].includes(f.role)).map(f => ({ role: f.role, size: f.expectedSize, sha256: f.expectedHash })) };
  assert.ok(matchesOfficialModel(entry, option));
  assert.equal(modelChoices("asr", [entry], options, []).length, 2);
  entry.modelFiles[0].sha256 = "c".repeat(64);
  assert.equal(matchesOfficialModel(entry, option), false);
});
test("one profile cannot select two different ASR backends", () => {
  const component = { roles: ["asr"], packageId: "33333333-3333-4333-8333-333333333333", modelName: "model", modelVersion: "fixed", checkOperationId: "44444444-4444-4444-8444-444444444444", settingsMatch: true, compatibilityCurrent: true };
  assert.throws(() => assertProfileView({ name: "standard", id: "55555555-5555-4555-8555-555555555555", profileVersion: 1, configurationComplete: false,
    components: [{ ...component, engine: "whisper.cpp" }, { ...component, engine: "asr" }] }));
});
