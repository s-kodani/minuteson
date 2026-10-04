import test from "node:test";
import assert from "node:assert/strict";
import { InspectionAttempt, inspectionLabel, type ModelInspectionReport } from "../src/model-inspection-state.ts";

const report: ModelInspectionReport = { manifest: { modelName: "fixture", modelVersion: "1", engine: "whisper.cpp", engineVersion: "test", target: "macos_aarch64", licenseRef: "test only", quantization: null }, manifestHash: "a".repeat(64), files: [], totalBytes: 0, engineCompatibility: "not_verified", publisherAuthenticity: "not_verified", installed: false, ready: false };
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
