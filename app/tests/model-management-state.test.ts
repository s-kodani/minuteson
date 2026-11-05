import test from "node:test";
import assert from "node:assert/strict";
import { assertModelDeletionPreview, type ModelDeletionPreview } from "../src/model-deletion-operation-state.ts";
import { acceptDataSnapshot, assertDataSnapshot, dataRequest, type DataSnapshot } from "../src/data-operation-state.ts";
const packageId = "11111111-1111-4111-8111-111111111111", operationId = "22222222-2222-4222-8222-222222222222";
const preview = (): ModelDeletionPreview => ({ confirmationId: "33333333-3333-4333-8333-333333333333", operationId, packageId, reviewHash: "a".repeat(64), modelName: "合成モデル", modelVersion: "1", modelsRoot: "/synthetic/models", phase: "review", pending: false, executeAllowed: true, resumeReason: null, keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: null, pathCount: 1, processedPaths: 0, fileCount: 1, missingCount: 0, changedCount: 0, unregisteredCount: 0, bytes: 10, profileReferences: 1, jobReferences: 0, activeJobs: 0, previousProcessedPaths: null, previousPathCount: null, priorDeletionCompleted: false, reservationRoots: [] });
const snapshot = (p = preview()): DataSnapshot => ({ revision: 1, token: operationId, kind: "model_deletion", phase: "ready", busy: false, backupPreview: null, restorePreview: null, backupReceipt: null, restoreReceipt: null, relocationPreview: null, relocationReceipt: null, deletionPreview: null, deletionReceipt: null, audioDeletionPreview: null, audioDeletionReceipt: null, modelDeletionPreview: p, modelDeletionReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: true });
test("selected model deletion requires bound confirmation and explicit unknown-file consent", () => {
  const s = snapshot(); assertDataSnapshot(s);
  assert.deepEqual(dataRequest(s, "publish"), { token: operationId, confirmationId: s.modelDeletionPreview!.confirmationId, action: "publish" });
  s.modelDeletionPreview!.unregisteredCount = 1;
  assert.throws(() => dataRequest(s, "publish"));
  assert.equal(dataRequest(s, "delete_including_unregistered").action, "delete_including_unregistered");
  s.phase = "failed"; assert.throws(() => dataRequest(s, "delete_including_unregistered"));
  assert.equal(dataRequest(s, "refresh").action, "refresh");
});
test("partial deletion admits only the exact operation's reserved roots after fresh consent", () => {
  const p = preview(); Object.assign(p, { phase: "reconfirmation", pending: true, previousProcessedPaths: 1, previousPathCount: 2, reservationRoots: [`.mon-delete-model-${operationId}-g2-0`] });
  const invalid = structuredClone(p); invalid.reservationRoots = [`.mon-delete-model-${packageId}-0`]; assert.throws(() => assertModelDeletionPreview(invalid));
  invalid.reservationRoots = [`.mon-delete-model-${operationId}-g33-0`]; assert.throws(() => assertModelDeletionPreview(invalid));
  const s = snapshot(p); s.attentionRequired = true; s.phase = "attention"; s.retryAllowed = false;
  assert.throws(() => dataRequest(s, "publish")); assert.equal(dataRequest(s, "refresh").action, "refresh"); assert.throws(() => dataRequest(s, "keep"));
});
test("completed deletion receipts must match the exact copy, operation and terminal phase", () => {
  const p = preview(); Object.assign(p, { phase: "completed", processedPaths: 1, executeAllowed: false, includeUnregisteredRecorded: false });
  const s = snapshot(p); Object.assign(s, { phase: "completed", retryAllowed: false, modelDeletionReceipt: { operationId, packageId, bodyDeleted: true, retained: false, historyRetained: true, restartRequired: false } });
  assertDataSnapshot(s); assert.throws(() => dataRequest(s, "publish"));
  s.modelDeletionReceipt!.packageId = operationId; assert.throws(() => assertDataSnapshot(s));
});

test("native model review transitions use model-specific progress and reject an audio phase", () => {
  const ready = snapshot();
  const choosing: DataSnapshot = { ...ready, phase: "choosing", busy: true, retryAllowed: false, modelDeletionPreview: null };
  const verifying = { ...choosing, revision: 2, phase: "verifying_model_delete_scope" };
  assert.equal(acceptDataSnapshot(choosing, verifying).phase, "verifying_model_delete_scope");
  assert.throws(() => acceptDataSnapshot(choosing, { ...verifying, phase: "verifying_audio_delete_scope" }));
  assert.equal(acceptDataSnapshot(verifying, { ...ready, revision: 3 }).phase, "ready");
});

test("native deletion results include the exact package root alongside reserved roots", () => {
  for (const phase of ["files", "awaiting_ack", "completed", "reconfirmation"] as const) {
    const p = preview();
    Object.assign(p, { phase, pending: phase !== "completed", executeAllowed: phase !== "completed", includeUnregisteredRecorded: phase === "reconfirmation" ? null : false,
      processedPaths: ["awaiting_ack", "completed"].includes(phase) ? 1 : 0,
      reservationRoots: [`.mon-delete-model-${operationId}-0`, packageId],
      ...(phase === "reconfirmation" ? { previousProcessedPaths: 0, previousPathCount: 1 } : {}) });
    assertModelDeletionPreview(p);
    for (const invalidRoots of [[operationId], [packageId, packageId], [`${packageId}/child`], [`.mon-delete-model-${packageId}-0`, packageId]]) {
      assert.throws(() => assertModelDeletionPreview({ ...p, reservationRoots: invalidRoots }));
    }
    if (phase === "completed") {
      const s = snapshot(p); Object.assign(s, { phase: "completed", retryAllowed: false, modelDeletionReceipt: { operationId, packageId, bodyDeleted: true, retained: false, historyRetained: true, restartRequired: false } });
      assertDataSnapshot(s);
    }
  }
});

test("native keep receipt has no deletion policy and never authorizes deleting files", () => {
  const p = preview(); Object.assign(p, { phase: "retained", keepAllowed: true, executeAllowed: false });
  const s = snapshot(p); Object.assign(s, { phase: "completed", retryAllowed: false, modelDeletionReceipt: { operationId, packageId, bodyDeleted: false, retained: true, historyRetained: true, restartRequired: false } });
  assertDataSnapshot(s);
  assert.throws(() => dataRequest(s, "publish"));
});
