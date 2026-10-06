import test from "node:test";
import assert from "node:assert/strict";
import { acceptDataSnapshot, assertDataSnapshot, dataRequest, type DataSnapshot } from "../src/data-operation-state.ts";
import { assertModelBodyDeletionListing, assertModelBodyDeletionPage, assertModelBodyDeletionPreview, modelDeletionStartupLabel, type ModelBodyDeletionPreview, type ModelDeletionStartupReason } from "../src/model-deletion-operation-state.ts";
const id = (n: number) => `${n}1111111-1111-4111-8111-111111111111`;
const preview = (): ModelBodyDeletionPreview => ({ confirmationId: id(2), operationId: id(3), packageId: id(4), reviewHash: "a".repeat(64), modelName: "合成モデル", modelVersion: "1", modelsRoot: "/synthetic/models", phase: "review", pending: false, executeAllowed: true, resumeReason: null, keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: null, pathCount: 2, processedPaths: 0, fileCount: 1, missingCount: 0, changedCount: 0, unregisteredCount: 0, bytes: 4, profileReferences: 1, jobReferences: 2, activeJobs: 0, previousProcessedPaths: null, previousPathCount: null, priorDeletionCompleted: false, reservationRoots: [] });
const ready = (): DataSnapshot => ({ revision: 2, token: id(1), kind: "model_deletion", phase: "ready", busy: false, backupPreview: null, backupReceipt: null, restorePreview: null, restoreReceipt: null, recoveryPreview: null, recoveryReceipt: null, relocationPreview: null, relocationReceipt: null, deletionPreview: null, deletionReceipt: null, modelDeletionPreview: preview(), modelDeletionReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: true });
test("model requests contain only server IDs and cannot publish an unknown fresh scope implicitly", () => {
  const s = ready(); assert.deepEqual(dataRequest(s, "publish"), { token: id(1), confirmationId: id(2), action: "publish" });
  assert.throws(() => dataRequest(s, "keep")); assert.throws(() => dataRequest(s, "recover")); assert.throws(() => dataRequest(s, "delete_including_unregistered"));
  s.modelDeletionPreview!.unregisteredCount = 1;
  assert.throws(() => dataRequest(s, "publish")); assert.equal(dataRequest(s, "delete_including_unregistered").action, "delete_including_unregistered");
});
test("unstarted continuation preserves the saved policy instead of asking to change it", () => {
  const s = ready(); Object.assign(s.modelDeletionPreview!, { phase: "not_started", pending: true, keepAllowed: true, unregisteredCount: 1, includeUnregisteredRecorded: true });
  s.restartRequired = true;
  assert.doesNotThrow(() => assertDataSnapshot(s)); assert.doesNotThrow(() => dataRequest(s, "publish")); assert.doesNotThrow(() => dataRequest(s, "keep"));
  assert.throws(() => dataRequest(s, "delete_including_unregistered"));
  assert.throws(() => assertDataSnapshot({ ...s, modelDeletionPreview: { ...s.modelDeletionPreview!, includeUnregisteredRecorded: null } }));
});
test("partial model deletion allows reinspection, never keep or direct retry from attention", () => {
  const s = ready(); Object.assign(s.modelDeletionPreview!, { phase: "files", pending: true, executeAllowed: false, resumeReason: "changed", reconfirmationAllowed: true, includeUnregisteredRecorded: false });
  Object.assign(s, { phase: "attention", attentionRequired: true, retryAllowed: false, restartRequired: true });
  assert.doesNotThrow(() => assertDataSnapshot(s)); assert.doesNotThrow(() => dataRequest(s, "refresh")); assert.doesNotThrow(() => dataRequest(s, "review_remaining"));
  assert.throws(() => dataRequest(s, "publish")); assert.throws(() => dataRequest(s, "keep"));
});
test("reconfirmation displays former progress and exact historical roots with a new unknown choice", () => {
  const s = ready(); Object.assign(s.modelDeletionPreview!, { phase: "reconfirmation", pending: true, unregisteredCount: 1, previousProcessedPaths: 2, previousPathCount: 5, priorDeletionCompleted: true, reservationRoots: [`.mon-delete-model-${id(3)}-g1-0`, id(4)] });
  assert.doesNotThrow(() => assertDataSnapshot(s)); assert.throws(() => dataRequest(s, "publish")); assert.doesNotThrow(() => dataRequest(s, "delete_including_unregistered"));
  for (const root of [`.mon-delete-model-${id(5)}-0`, `.mon-delete-model-${id(3)}-g33-0`, `.mon-delete-model-${id(3)}-256`, "../outside", id(5)]) assert.throws(() => assertModelBodyDeletionPreview({ ...s.modelDeletionPreview!, reservationRoots: [root] }));
  assert.throws(() => assertModelBodyDeletionPreview({ ...s.modelDeletionPreview!, previousProcessedPaths: 6 }));
});
test("model completion binds package, original operation, retained history and restart", () => {
  const s = ready(); Object.assign(s.modelDeletionPreview!, { phase: "completed", executeAllowed: false, processedPaths: 2, includeUnregisteredRecorded: false, priorDeletionCompleted: true, reservationRoots: [id(4)] });
  Object.assign(s, { phase: "completed", retryAllowed: false, restartRequired: true });
  s.modelDeletionReceipt = { operationId: id(3), packageId: id(4), bodyDeleted: true, retained: false, historyRetained: true, restartRequired: true };
  assert.doesNotThrow(() => assertDataSnapshot(s)); assert.throws(() => dataRequest(s, "publish"));
  for (const change of [{ packageId: id(5) }, { operationId: id(5) }, { historyRetained: false }, { retained: true }, { restartRequired: false }]) assert.throws(() => assertDataSnapshot({ ...s, modelDeletionReceipt: { ...s.modelDeletionReceipt!, ...change } } as DataSnapshot));
  assert.throws(() => assertDataSnapshot({ ...s, busy: true }));
});
test("model keep is terminal without pretending body removal or accepting deletion progress phases for meetings", () => {
  const s = ready(); Object.assign(s.modelDeletionPreview!, { phase: "retained", executeAllowed: false, keepAllowed: true }); Object.assign(s, { phase: "completed", retryAllowed: false });
  s.modelDeletionReceipt = { operationId: id(3), packageId: id(4), bodyDeleted: false, retained: true, historyRetained: true, restartRequired: false };
  assert.doesNotThrow(() => assertDataSnapshot(s)); assert.throws(() => dataRequest(s, "keep"));
  for (const phase of ["verifying_model_delete_scope", "fixing_model_delete_scope", "model_delete_scope_fixed", "deleting_model_files", "model_delete_completed"]) {
    const busy = { ...ready(), phase, busy: true, retryAllowed: false }; assert.doesNotThrow(() => assertDataSnapshot(busy)); assert.throws(() => assertDataSnapshot({ ...busy, kind: "deletion" }));
  }
});
test("model path pages bind frozen confirmation, root, page size, missing state and hashes", () => {
  const p = preview();
  const page = { confirmationId: p.confirmationId, offset: 0, total: 2, nextOffset: null, entries: [
    { relativePath: p.packageId, directory: true, registered: true, present: true, bytes: null, sha256: null, changed: false },
    { relativePath: p.packageId + "/cli", directory: false, registered: true, present: true, bytes: 4, sha256: "b".repeat(64), changed: false },
  ] };
  assert.doesNotThrow(() => assertModelBodyDeletionPage(page, p, 0));
  for (const change of [{ relativePath: id(5) + "/cli" }, { relativePath: p.packageId + "/../cli" }, { relativePath: p.packageId + "//cli" }, { sha256: null }, { present: false }, { bytes: -1 }]) assert.throws(() => assertModelBodyDeletionPage({ ...page, entries: [page.entries[0], { ...page.entries[1], ...change }] }, p, 0));
  assert.throws(() => assertModelBodyDeletionPage({ ...page, confirmationId: id(5) }, p, 0)); assert.throws(() => assertModelBodyDeletionPage({ ...page, entries: [page.entries[0]] }, p, 0));
});
test("pending model listing preserves unreadable operations instead of granting authority", () => {
  const listing = { items: [{ operationId: id(3), state: "prepared", packageId: id(4), modelName: "合成モデル", reviewAllowed: true, errorMessage: null }], writeBlocked: true, restartRequired: true, reason: "pending" };
  assert.doesNotThrow(() => assertModelBodyDeletionListing(listing));
  const bad = { operationId: id(5), state: "installing", packageId: null, modelName: null, reviewAllowed: false, errorMessage: "cannot verify" };
  assert.doesNotThrow(() => assertModelBodyDeletionListing({ ...listing, items: [bad] }));
  assert.throws(() => assertModelBodyDeletionListing({ ...listing, items: [{ ...bad, reviewAllowed: true }] }));
  assert.throws(() => assertModelBodyDeletionListing({ ...listing, items: [listing.items[0], listing.items[0]] }));
});
test("stale model snapshots cannot replace a newer server confirmation", () => {
  const s = ready(), newer = { ...ready(), revision: 3, modelDeletionPreview: { ...preview(), confirmationId: id(5) } };
  assert.equal(acceptDataSnapshot(newer, s), newer); assert.throws(() => acceptDataSnapshot(s, { ...s, modelDeletionPreview: { ...preview(), confirmationId: id(5) } }));
});
test("completed model startup reasons require a completed record and write protection", () => {
  for (const startupReason of ["model_completed_scope_reappeared", "model_completed_receipt_invalid", "model_completed_scope_uninspectable"] as ModelDeletionStartupReason[]) {
    const item = { operationId: id(3), state: "completed", packageId: id(4), modelName: "合成モデル", reviewAllowed: true, errorMessage: null, startupReason };
    const listing = { items: [item], writeBlocked: true, restartRequired: false, reason: "attention" };
    assert.doesNotThrow(() => assertModelBodyDeletionListing(listing));
    assert.throws(() => assertModelBodyDeletionListing({ ...listing, writeBlocked: false, reason: null }));
    assert.throws(() => assertModelBodyDeletionListing({ ...listing, items: [{ ...item, state: "prepared" }] }));
    assert.match(modelDeletionStartupLabel(startupReason), /完了済みモデル削除/u);
  }
});
test("unknown startup reason never grants listing authority", () => {
  const listing = { items: [{ operationId: id(3), state: "completed", packageId: id(4), modelName: "合成モデル", reviewAllowed: true, errorMessage: null, startupReason: "automatic_cleanup" }], writeBlocked: true, restartRequired: false, reason: "attention" };
  assert.throws(() => assertModelBodyDeletionListing(listing as never));
});
test("unreadable completed startup evidence remains visible without review authority", () => {
  const item = { operationId: id(3), state: "completed", packageId: null, modelName: null, reviewAllowed: false, errorMessage: "evidence invalid", startupReason: "model_completed_receipt_invalid" as const };
  assert.doesNotThrow(() => assertModelBodyDeletionListing({ items: [item], writeBlocked: true, restartRequired: false, reason: "held" }));
  assert.match(modelDeletionStartupLabel(item.startupReason), /対象を保持/u);
});
