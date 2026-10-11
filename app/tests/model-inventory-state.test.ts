import test from "node:test";
import assert from "node:assert/strict";
import { installedModels, modelUsage } from "../src/model-inventory-state.ts";
import type { ModelPlacementEntry } from "../src/model-inspection-state.ts";
import type { ModelProfileView } from "../src/model-profile-state.ts";
const entry: ModelPlacementEntry = { packageId: "package", modelName: "model", modelVersion: "1", engine: "whisper.cpp", engineVersion: "1", totalBytes: 10, placementRecorded: true, ready: false, reviewHash: "hash", compatibility: { operationId: "check", packageId: "package", reviewHash: "hash", adapterRevision: 1, state: "succeeded", errorMessage: null } };
const view: ModelProfileView = { name: "standard", id: "profile", profileVersion: 1, configurationComplete: false, components: [{ roles: ["asr"], packageId: "package", modelName: "model", modelVersion: "1", engine: "whisper.cpp", checkOperationId: "check", settingsMatch: true, compatibilityCurrent: true }] };
test("inventory excludes unfinished imports and removed bodies, including restored metadata", () => {
  assert.deepEqual(installedModels([entry, { ...entry, packageId: "unfinished", placementRecorded: false }, { ...entry, packageId: "removed", bodyRemoved: true }]), [entry]);
});
test("usage requires exact current package, engine, check and settings evidence", () => {
  assert.equal(modelUsage(entry, [view]), "標準で使用");
  assert.equal(modelUsage(entry, [view, { ...view, name: "high_accuracy" }]), "標準で使用・高精度で使用");
  for (const change of [{ packageId: "other" }, { engine: "llama.cpp" as const }, { checkOperationId: "old" }, { settingsMatch: false }, { compatibilityCurrent: false }]) {
    assert.equal(modelUsage(entry, [{ ...view, components: [{ ...view.components[0], ...change }] }]), "動作確認済み・未選択");
  }
});
test("missing data and failed/pending checks never imply selected or ready", () => {
  assert.equal(modelUsage(entry, null), "利用状態を確認できません");
  assert.equal(modelUsage({ ...entry, compatibility: null }, [view]), "動作確認待ち");
  assert.equal(modelUsage({ ...entry, compatibility: { ...entry.compatibility!, state: "failed" } }, [view]), "動作確認に失敗");
  assert.equal(modelUsage({ ...entry, bodyRemoved: true }, [view]), "本体なし");
  assert.equal(modelUsage({ ...entry, placementRecorded: false }, [view]), "取り込み未完了");
});
