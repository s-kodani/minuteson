import test from "node:test";
import assert from "node:assert/strict";
import { applicationChanges, assertApplicationPreview, assertApplicationResult, assertAppliedPreferences, assertProfileCatalog, canReviewApplication, preferenceCommand, profileConfigurationLabel, type ModelApplicationPreview, type ModelProfileComponent, type ModelProfileView } from "../src/model-profile-state.ts";
import type { ModelPlacementEntry } from "../src/model-inspection-state.ts";

const packageId = "11111111-1111-4111-8111-111111111111", checkId = "22222222-2222-4222-8222-222222222222", operationId = "33333333-3333-4333-8333-333333333333";
const empty: ModelProfileView = { name: "standard", id: null, profileVersion: 0, components: [], configurationComplete: false };
const entry: ModelPlacementEntry = { packageId, modelName: "fixture", modelVersion: "1", engine: "whisper.cpp", engineVersion: "declared", totalBytes: 10, placementRecorded: true, ready: false, reviewHash: "a".repeat(64), compatibility: { operationId: checkId, packageId, reviewHash: "a".repeat(64), adapterRevision: 1, state: "succeeded", errorMessage: null } };
const before = { engine: "whisper.cpp" as const, settings: { executablePath: "", modelPath: "", vadModelPath: null, threads: 7, useGpu: true } };
const after = { engine: "whisper.cpp" as const, settings: { ...before.settings, executablePath: "/app/models/cli", modelPath: "/app/models/model.bin" } };
const preview: ModelApplicationPreview = { schemaVersion: 1, profile: "standard", packageId, reviewHash: entry.reviewHash, checkOperationId: checkId, expectedProfileId: null, expectedProfileHash: null, expectedProfileVersion: 0, expectedPreferencesVersion: 5, before, after, reviewDigest: "b".repeat(64) };
const component: ModelProfileComponent = { roles: ["asr"], packageId, modelName: "fixture", modelVersion: "1", engine: "whisper.cpp", checkOperationId: checkId, settingsMatch: true, compatibilityCurrent: true };
const applied: ModelProfileView = { ...empty, id: operationId, profileVersion: 1, components: [component] };

test("application eligibility requires a placed package and the latest successful bound CPU record", () => {
  assert.ok(canReviewApplication(entry));
  assert.equal(canReviewApplication({ ...entry, compatibility: null }), false);
  assert.equal(canReviewApplication({ ...entry, placementRecorded: false, compatibility: null }), false);
  for (const state of ["preparing", "failed", "cancelled", "interrupted"] as const) {
    assert.equal(canReviewApplication({ ...entry, compatibility: { ...entry.compatibility!, state, errorMessage: state === "preparing" ? null : "fixture reason" } }), false);
  }
  assert.throws(() => canReviewApplication({ ...entry, compatibility: { ...entry.compatibility!, packageId: operationId } }));
});
test("catalog validation rejects duplicate profiles, missing components, wrong roles and false readiness claims", () => {
  assert.doesNotThrow(() => assertProfileCatalog([empty, { ...empty, name: "high_accuracy" }]));
  assert.doesNotThrow(() => assertProfileCatalog([applied, { ...empty, name: "high_accuracy" }]));
  for (const views of [[empty], [empty, empty], [{ ...empty, configurationComplete: true }, { ...empty, name: "high_accuracy" }], [{ ...applied, components: [] }, { ...empty, name: "high_accuracy" }], [{ ...applied, components: [component, component] }, { ...empty, name: "high_accuracy" }], [{ ...applied, components: [{ ...component, roles: ["llm"] }] }, { ...empty, name: "high_accuracy" }]]) {
    assert.throws(() => assertProfileCatalog(views as ModelProfileView[]));
  }
});
test("partial and complete configuration labels remain distinct from runtime preparation and expose stale flags", () => {
  assert.match(profileConfigurationLabel(empty), /未選択/);
  assert.match(profileConfigurationLabel(applied), /一部/);
  const complete: ModelProfileView = { ...applied, configurationComplete: true, components: [component, { ...component, engine: "sherpa-onnx", roles: ["segmentation", "speaker_embedding"] }, { ...component, engine: "llama.cpp", roles: ["llm"] }] };
  assert.match(profileConfigurationLabel(complete), /実行準備・品質は別途確認/);
  for (const flag of ["settingsMatch", "compatibilityCurrent"]) {
    assert.match(profileConfigurationLabel({ ...complete, configurationComplete: false, components: complete.components.map((c, i) => i === 0 ? { ...c, [flag]: false } : c) }), /不一致/);
  }
});
test("review binds selected profile package check and head without replacing prior numeric or GPU conditions", () => {
  assert.doesNotThrow(() => assertApplicationPreview(preview, entry, "standard", empty));
  const changes = applicationChanges(preview);
  assert.equal(changes.find(c => c.key === "useGpu")?.after, true);
  assert.equal(changes.find(c => c.key === "threads")?.after, 7);
  for (const patch of [{ schemaVersion: 2 }, { profile: "high_accuracy" }, { packageId: operationId }, { reviewHash: "c".repeat(64) }, { checkOperationId: operationId }, { expectedProfileId: operationId }, { expectedProfileHash: "c".repeat(64) }, { expectedProfileVersion: 1 }, { expectedPreferencesVersion: -1 }, { reviewDigest: "bad" }, { after: { ...after, settings: { ...after.settings, threads: 1 } } }, { after: { ...after, settings: { ...after.settings, useGpu: false } } }, { after: { ...after, settings: { ...after.settings, modelPath: "relative" } } }, { after: { ...after, settings: { ...after.settings, extra: 1 } } }]) {
    assert.throws(() => assertApplicationPreview({ ...preview, ...patch } as ModelApplicationPreview, entry, "standard", empty));
  }
  assert.throws(() => assertApplicationPreview(preview, entry, "standard", applied));
});
test("high accuracy and both sherpa model roles keep all tuning values and show runtime library paths", () => {
  const settings = { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [] as string[], threads: 6, clusterThresholdMilli: 700, windowShiftMilli: 400, minDurationOnMs: 200, minDurationOffMs: 600 };
  const p: ModelApplicationPreview = { ...preview, profile: "high_accuracy", before: { engine: "sherpa-onnx", settings }, after: { engine: "sherpa-onnx", settings: { ...settings, executablePath: "/app/models/sherpa", segmentationModelPath: "/app/models/seg.onnx", embeddingModelPath: "/app/models/emb.onnx", runtimeLibraryPaths: ["/app/models/lib.dylib"] } } };
  assert.doesNotThrow(() => assertApplicationPreview(p, { ...entry, engine: "sherpa-onnx" }, "high_accuracy", { ...empty, name: "high_accuracy" }));
  assert.deepEqual(applicationChanges(p).find(c => c.key === "runtimeLibraryPaths")?.after, ["/app/models/lib.dylib"]);
  assert.throws(() => assertApplicationPreview({ ...p, after: { ...p.after, settings: { ...p.after.settings, clusterThresholdMilli: 900 } } } as ModelApplicationPreview, { ...entry, engine: "sherpa-onnx" }, "high_accuracy", { ...empty, name: "high_accuracy" }));
});
test("llama review retains seed context input and output budgets rather than CPU trial presets", () => {
  const settings = { executablePath: "", modelPath: "", threads: 5, contextTokens: 8192, predictionTokens: 512, gpuLayers: 2, seed: 777, promptBudget: 5000 };
  const p: ModelApplicationPreview = { ...preview, before: { engine: "llama.cpp", settings }, after: { engine: "llama.cpp", settings: { ...settings, executablePath: "/app/models/llama", modelPath: "/app/models/model.gguf" } } };
  assert.doesNotThrow(() => assertApplicationPreview(p, { ...entry, engine: "llama.cpp" }, "standard", empty));
  for (const key of ["threads", "contextTokens", "predictionTokens", "gpuLayers", "seed", "promptBudget"]) {
    assert.throws(() => assertApplicationPreview({ ...p, after: { ...p.after, settings: { ...p.after.settings, [key]: 1 } } } as ModelApplicationPreview, { ...entry, engine: "llama.cpp" }, "standard", empty));
  }
});
test("applied response must bind the same explicit operation revision package check and matching current settings", () => {
  assert.doesNotThrow(() => assertApplicationResult(applied, operationId, preview));
  for (const patch of [{ id: packageId }, { name: "high_accuracy" }, { profileVersion: 2 }, { components: [{ ...component, packageId: operationId }] }, { components: [{ ...component, checkOperationId: operationId }] }, { components: [{ ...component, settingsMatch: false }] }, { components: [{ ...component, compatibilityCurrent: false }] }]) {
    assert.throws(() => assertApplicationResult({ ...applied, ...patch } as ModelProfileView, operationId, preview));
  }
});
test("readback checks the selected actual preferences and never substitutes settings from the other profile", () => {
  const value = { lockVersion: 6, payload: { standard: after.settings, highAccuracy: before.settings } };
  assert.doesNotThrow(() => assertAppliedPreferences(value, preview));
  assert.throws(() => assertAppliedPreferences({ ...value, lockVersion: 5 }, preview));
  assert.throws(() => assertAppliedPreferences({ ...value, payload: { standard: before.settings, highAccuracy: after.settings } }, preview));
  assert.throws(() => assertAppliedPreferences({ ...value, payload: { ...value.payload, standard: { ...after.settings, threads: 1 } } }, preview));
  assert.equal(preferenceCommand("whisper.cpp"), "get_analysis_preferences");
  assert.equal(preferenceCommand("sherpa-onnx"), "get_diarization_preferences");
  assert.equal(preferenceCommand("llama.cpp"), "get_minutes_preferences");
});
test("CPU tuning requires the explicit requested thread count and cannot change model files or diarization conditions", () => {
  const settings = { executablePath: "/app/models/sherpa", segmentationModelPath: "/app/models/seg.onnx", embeddingModelPath: "/app/models/emb.onnx", runtimeLibraryPaths: ["/app/models/lib.dylib"], threads: 4, clusterThresholdMilli: 500, windowShiftMilli: 100, minDurationOnMs: 0, minDurationOffMs: 0 };
  const current: ModelProfileView = { ...applied, components: [{ ...component, engine: "sherpa-onnx", roles: ["segmentation", "speaker_embedding"] }] };
  const p: ModelApplicationPreview = { ...preview, expectedProfileId: current.id, expectedProfileHash: "c".repeat(64), expectedProfileVersion: current.profileVersion, before: { engine: "sherpa-onnx", settings }, after: { engine: "sherpa-onnx", settings: { ...settings, threads: 8 } }, diarizationThreads: 8 };
  const e = { ...entry, engine: "sherpa-onnx" as const };
  assert.doesNotThrow(() => assertApplicationPreview(p, e, "standard", current, 8));
  assert.throws(() => assertApplicationPreview(p, e, "standard", current));
  assert.throws(() => assertApplicationPreview(p, e, "standard", current, 4));
  assert.throws(() => assertApplicationPreview(p, e, "standard", { ...current, components: [{ ...current.components[0], settingsMatch: false }] }, 8));
  for (const patch of [{ threads: 2 }, { segmentationModelPath: "/app/models/other.onnx" }, { embeddingModelPath: "/app/models/other.onnx" }, { clusterThresholdMilli: 900 }, { windowShiftMilli: 500 }, { minDurationOnMs: 300 }, { minDurationOffMs: 500 }]) {
    assert.throws(() => assertApplicationPreview({ ...p, after: { engine: "sherpa-onnx", settings: { ...settings, threads: 8, ...patch } } }, e, "standard", current, 8));
  }
  assert.throws(() => assertApplicationPreview({ ...preview, diarizationThreads: 8 }, entry, "standard", empty, 8));
});
