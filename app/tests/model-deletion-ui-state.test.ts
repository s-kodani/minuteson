import test from "node:test";
import assert from "node:assert/strict";
import { modelDeletionNewScope, modelDeletionPathProcessed, modelDeletionPublishRequest } from "../src/model-deletion-ui-state.ts";
import type { DataSnapshot } from "../src/data-operation-state.ts";
import type { ModelBodyDeletionPage, ModelBodyDeletionPreview } from "../src/model-deletion-operation-state.ts";
const id = (n: number) => `${n}1111111-1111-4111-8111-111111111111`;
const preview = (): ModelBodyDeletionPreview => ({ confirmationId: id(2), operationId: id(3), packageId: id(4), reviewHash: "a".repeat(64), modelName: "合成", modelVersion: "1", modelsRoot: "/synthetic/models", phase: "review", pending: false, executeAllowed: true, resumeReason: null, keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: null, pathCount: 1, processedPaths: 0, fileCount: 1, missingCount: 0, changedCount: 0, unregisteredCount: 0, bytes: 4, profileReferences: 0, jobReferences: 0, activeJobs: 0, previousProcessedPaths: null, previousPathCount: null, priorDeletionCompleted: false, reservationRoots: [] });
const ready = (p = preview()): DataSnapshot => ({ revision: 2, token: id(1), kind: "model_deletion", phase: "ready", busy: false, backupPreview: null, backupReceipt: null, restorePreview: null, restoreReceipt: null, recoveryPreview: null, recoveryReceipt: null, relocationPreview: null, relocationReceipt: null, modelDeletionPreview: p, modelDeletionReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: true });
const page = (p: ModelBodyDeletionPreview): ModelBodyDeletionPage => ({ confirmationId: p.confirmationId, offset: 0, total: 1, nextOffset: null, entries: [{ relativePath: p.packageId + "/cli", registered: true, present: true, directory: false, bytes: 4, sha256: "b".repeat(64), changed: false }] });
test("model UI publication binds consent, frozen page and current confirmation", () => {
  const s = ready(), p = s.modelDeletionPreview!;
  assert.equal(modelDeletionPublishRequest(s, p.confirmationId, page(p), p.confirmationId, null).action, "publish");
  for (const [confirmation, understood] of [[id(5), p.confirmationId], [p.confirmationId, id(5)], [p.confirmationId, null]]) assert.throws(() => modelDeletionPublishRequest(s, confirmation!, page(p), understood, null));
  assert.throws(() => modelDeletionPublishRequest(s, p.confirmationId, null, p.confirmationId, null));
  assert.throws(() => modelDeletionPublishRequest(s, p.confirmationId, { ...page(p), confirmationId: id(5) }, p.confirmationId, null));
});
test("fresh unknown scope requires separate current consent and exact action", () => {
  const p = { ...preview(), unregisteredCount: 1 }, s = ready(p), paths = page(p); paths.entries[0].registered = false;
  assert.throws(() => modelDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, null));
  assert.throws(() => modelDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, id(5)));
  assert.equal(modelDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, p.confirmationId).action, "delete_including_unregistered");
});
test("saved unstarted scope uses recorded policy, not a new unknown action", () => {
  const p: ModelBodyDeletionPreview = { ...preview(), phase: "not_started", pending: true, keepAllowed: true, unregisteredCount: 1, includeUnregisteredRecorded: true }, s = ready(p), paths = page(p); paths.entries[0].registered = false;
  assert.equal(modelDeletionNewScope(p), false);
  assert.equal(modelDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, null).action, "publish");
  assert.equal(modelDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, p.confirmationId).action, "publish");
});
test("new remaining scope never inherits earlier consent", () => {
  const p: ModelBodyDeletionPreview = { ...preview(), confirmationId: id(5), phase: "reconfirmation", pending: true, previousProcessedPaths: 1, previousPathCount: 2, unregisteredCount: 1 }, s = ready(p), paths = page(p); paths.entries[0].registered = false;
  assert.equal(modelDeletionNewScope(p), true);
  assert.throws(() => modelDeletionPublishRequest(s, p.confirmationId, paths, id(2), id(2)));
  assert.equal(modelDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, p.confirmationId).action, "delete_including_unregistered");
});
test("busy, attention, wrong kind, missing execution permission and foreign paths cannot publish", () => {
  const p = preview(), s = ready(p);
  for (const changed of [{ busy: true }, { phase: "attention", attentionRequired: true, retryAllowed: false }, { kind: "backup" }, { modelDeletionPreview: { ...p, executeAllowed: false, resumeReason: "changed" } }]) assert.throws(() => modelDeletionPublishRequest({ ...s, ...changed } as DataSnapshot, p.confirmationId, page(p), p.confirmationId, null));
  const paths = page(p); paths.entries[0].relativePath = id(5) + "/cli";
  assert.throws(() => modelDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, null));
});
test("processed markers follow reverse frozen cursor including directory/missing entries", () => {
  const p = { ...preview(), pathCount: 105, processedPaths: 3 };
  assert.equal(modelDeletionPathProcessed(p, 101), false);
  for (const index of [102, 103, 104]) assert.equal(modelDeletionPathProcessed(p, index), true);
  for (const index of [-1, 0, 105]) assert.equal(modelDeletionPathProcessed(p, index), false);
});
