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
  assert.equal(preferenceCommand("whisper.cpp"), "get_asr_preferences");
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

test("automatic merge tuning binds the selected model and exact threshold without changing CPU or other conditions", () => {
  const settings = { executablePath: "/app/models/sherpa", segmentationModelPath: "/app/models/seg.onnx", embeddingModelPath: "/app/models/emb.onnx", runtimeLibraryPaths: ["/app/models/lib.dylib"], threads: 8, clusterThresholdMilli: 500, windowShiftMilli: 100, minDurationOnMs: 0, minDurationOffMs: 0 };
  const current: ModelProfileView = { ...applied, components: [{ ...component, engine: "sherpa-onnx", roles: ["segmentation", "speaker_embedding"] }] };
  const p: ModelApplicationPreview = { ...preview, expectedProfileId: current.id, expectedProfileHash: "c".repeat(64), expectedProfileVersion: current.profileVersion, before: { engine: "sherpa-onnx", settings }, after: { engine: "sherpa-onnx", settings: { ...settings, clusterThresholdMilli: 1300 } }, diarizationClusterThresholdMilli: 1300 };
  const e = { ...entry, engine: "sherpa-onnx" as const };
  assert.doesNotThrow(() => assertApplicationPreview(p,e,"standard",current,undefined,1300));
  assert.throws(() => assertApplicationPreview(p,e,"standard",current));
  for (const threshold of [-1,2001,1.3,1500,NaN]) assert.throws(() => assertApplicationPreview(p,e,"standard",current,undefined,threshold));
  for (const patch of [{threads:4},{clusterThresholdMilli:1500},{minDurationOnMs:300},{windowShiftMilli:500},{embeddingModelPath:"/app/models/other.onnx"}]) {
    assert.throws(() => assertApplicationPreview({...p,after:{engine:"sherpa-onnx",settings:{...settings,clusterThresholdMilli:1300,...patch}}},e,"standard",current,undefined,1300));
  }
  assert.throws(() => assertApplicationPreview(p,e,"standard",{...current,components:[{...current.components[0],settingsMatch:false}]},undefined,1300));
  assert.throws(() => assertApplicationPreview({...preview,diarizationClusterThresholdMilli:1300},entry,"standard",empty,undefined,1300));
  assert.doesNotThrow(() => assertAppliedPreferences({lockVersion:6,payload:{standard:p.after.settings,highAccuracy:settings}},p));
  assert.throws(() => assertAppliedPreferences({lockVersion:6,payload:{standard:settings,highAccuracy:p.after.settings}},p));
});

test("VAD options bind the request and readback without accepting changes to other settings", () => {
  const options = { enabled:false,thresholdMilli:750,minSpeechDurationMs:450,minSilenceDurationMs:100,maxSpeechDurationS:30,speechPadMs:30,samplesOverlapMs:100 };
  const e = {...entry,modelFiles:[{role:"vad",size:885098,sha256:"a".repeat(64)}]};
  const p: ModelApplicationPreview = {...preview,whisperVadOptions:options,after:{engine:"whisper.cpp",settings:{...after.settings,vadModelPath:"/app/models/vad",vadOptions:options}}};
  assert.doesNotThrow(()=>assertApplicationPreview(p,e,"standard",empty,undefined,undefined,options));
  assert.throws(()=>assertApplicationPreview(p,e,"standard",empty));
  assert.throws(()=>assertApplicationPreview({...p,whisperVadOptions:{...options,thresholdMilli:800}},e,"standard",empty,undefined,undefined,options));
  assert.throws(()=>assertApplicationPreview({...p,after:{engine:"whisper.cpp",settings:{...after.settings,threads:1,vadOptions:options}}},e,"standard",empty,undefined,undefined,options));
  assert.throws(()=>assertApplicationPreview(p,{...e,engine:"llama.cpp"},"standard",empty,undefined,undefined,options));
  assert.doesNotThrow(()=>assertAppliedPreferences({lockVersion:6,payload:{standard:p.after.settings,highAccuracy:before.settings}},p));
});
