import test from "node:test";
import assert from "node:assert/strict";
import { InspectionAttempt, inspectionLabel, placementLabel, assertPlacementReceipt, type ModelInspectionReport, type ModelPlacementReceipt } from "../src/model-inspection-state.ts";

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
