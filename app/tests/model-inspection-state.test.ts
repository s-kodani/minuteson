import test from "node:test";
import assert from "node:assert/strict";
import { InspectionAttempt, inspectionLabel, placementLabel, assertPlacementReceipt, assertCompatibilitySummary, type ModelCompatibilitySummary, type ModelInspectionReport, type ModelPlacementReceipt, type ModelPlacementEntry } from "../src/model-inspection-state.ts";

const report: ModelInspectionReport = { manifest: { modelName: "fixture", modelVersion: "1", engine: "whisper.cpp", engineVersion: "test", target: "macos_aarch64", licenseRef: "test only", quantization: null }, manifestHash: "a".repeat(64), reviewHash: "r", files: [], totalBytes: 0, engineCompatibility: "not_verified", publisherAuthenticity: "not_verified", installed: false, ready: false };
test("a preflight never labels file hashes as AI readiness or publisher authentication", () => {
  assert.match(inspectionLabel(report), /未導入/);
  for (const field of ["installed", "ready", "engineCompatibility", "publisherAuthenticity"]) {
    assert.throws(() => inspectionLabel({ ...report, [field]: field === "installed" || field === "ready" ? true : "verified" } as unknown as ModelInspectionReport));
  }
});
test("cancelled and superseded inspection results cannot be displayed", () => {
  const attempt = new InspectionAttempt();
  assert.equal(attempt.cancel(), null);
  attempt.begin("first"); assert.ok(attempt.accepts("first"));
  assert.equal(attempt.cancel(), "first"); assert.equal(attempt.accepts("first"), false);
  attempt.begin("second"); assert.equal(attempt.accepts("first"), false);
  assert.equal(attempt.finish("first"), false); assert.ok(attempt.accepts("second"));
  assert.equal(attempt.finish("second"), true); assert.equal(attempt.accepts("second"), false);
});
test("placement is not readiness and its response must bind operation and reviewed bytes", () => {
  const receipt: ModelPlacementReceipt = { packageId: "operation", reviewHash: "review", totalBytes: 100, placementRecorded: true, engineCompatibility: "not_verified", ready: false };
  assert.doesNotThrow(() => assertPlacementReceipt(receipt, "operation", "review"));
  for (const patch of [{ packageId: "other" }, { reviewHash: "changed" }, { placementRecorded: false }, { ready: true }, { engineCompatibility: "verified" }]) {
    assert.throws(() => assertPlacementReceipt({ ...receipt, ...patch } as unknown as ModelPlacementReceipt, "operation", "review"));
  }
  assert.match(placementLabel(receipt), /検証待ち/);
  assert.match(placementLabel({ placementRecorded: false, ready: false }), /未完了/);
  assert.throws(() => placementLabel({ placementRecorded: true, ready: true } as never));
});
const entry: ModelPlacementEntry = { packageId: "11111111-1111-4111-8111-111111111111", modelName: "fixture", modelVersion: "1", engine: "llama.cpp", engineVersion: "declared only", totalBytes: 100, placementRecorded: true, ready: false, reviewHash: "a".repeat(64) };
const summary: ModelCompatibilitySummary = { operationId: "22222222-2222-4222-8222-222222222222", packageId: entry.packageId, reviewHash: entry.reviewHash, adapterRevision: 1, state: "succeeded", errorMessage: null };
test("a fixed CPU smoke result remains unapplied and all interrupted states require explicit action", () => {
  assert.match(placementLabel({ ...entry, compatibility: summary }), /CPU互換性確認済み・プロファイル未適用/);
  for (const state of ["preparing", "failed", "cancelled", "interrupted"] as const) {
    const compatibility = { ...summary, state, errorMessage: state === "preparing" ? null : "fixture reason" };
    assert.doesNotThrow(() => placementLabel({ ...entry, compatibility }));
  }
  for (const patch of [{ ready: true }, { placementRecorded: false }, { packageId: "other" }, { reviewHash: "other" }]) {
    assert.throws(() => placementLabel({ ...entry, ...patch, compatibility: summary } as never));
  }
});
test("runtime responses must bind the selected bytes, operation, preset and terminal outcome", () => {
  assert.doesNotThrow(() => assertCompatibilitySummary(summary, summary.operationId, entry));
  for (const state of ["failed", "cancelled"] as const) {
    assert.doesNotThrow(() => assertCompatibilitySummary({ ...summary, state, errorMessage: "fixture reason" }, summary.operationId, entry));
  }
  for (const patch of [{ operationId: entry.packageId }, { packageId: summary.operationId }, { reviewHash: "b".repeat(64) }, { reviewHash: "a".repeat(63) }, { adapterRevision: 2 }, { state: "preparing" }, { state: "interrupted", errorMessage: "reason" }, { state: "unknown" }, { errorMessage: "not success" }, { state: "failed" }, { state: "failed", errorMessage: "" }, { state: "failed", errorMessage: "x\0y" }, { state: "failed", errorMessage: "あ".repeat(1366) }]) {
    assert.throws(() => assertCompatibilitySummary({ ...summary, ...patch } as never, summary.operationId, entry));
  }
  const attempt = new InspectionAttempt();
  attempt.begin(summary.operationId); attempt.cancel();
  assert.equal(attempt.accepts(summary.operationId), false, "late success cannot override cancellation");
});
