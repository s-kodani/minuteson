import test from "node:test";
import assert from "node:assert/strict";
import { assertRecoveryListing, assertRecoveryPreview, assertRecoveryReceipt, recoveryOutcomeLabel, type RecoveryPreview } from "../src/recovery-state.ts";
import { dataRequest, assertDataSnapshot, type DataSnapshot } from "../src/data-operation-state.ts";
const id = "11111111-1111-4111-8111-111111111111";
const preview = (): RecoveryPreview => ({ confirmationId: id, item: { source: "scratch", operationId: id, paths: [`.mon-stage-${id}`], registeredPaths: 2, reason: "所有不明", retained: false }, root: "/safe/root", observedPaths: 3, inspectionComplete: true });
const ready = (): DataSnapshot => ({ revision: 1, token: id, kind: "recovery", phase: "ready", busy: false, retryAllowed: true, attentionRequired: false, errorMessage: null, backupPreview: null, restorePreview: null, backupReceipt: null, restoreReceipt: null, recoveryPreview: preview(), recoveryReceipt: null, relocationPreview: null, relocationReceipt: null, restartRequired: false });
test("recovery requests bind the native token and confirmation, not client paths", () => {
  for (const action of ["recover", "keep", "refresh"] as const) assert.deepEqual(dataRequest(ready(), action), { token: id, confirmationId: id, action });
  assert.throws(() => dataRequest(ready(), "publish"));
  for (const phase of ["failed", "cancelled"]) { const s = { ...ready(), phase }; assert.throws(() => dataRequest(s, "keep")); assert.throws(() => dataRequest(s, "recover")); assert.doesNotThrow(() => dataRequest(s, "refresh")); }
});
test("recovery list rejects foreign paths, duplicate identities, unsafe bounds and unsupported mutations", () => {
  const p = preview(); assert.doesNotThrow(() => assertRecoveryPreview(p));
  for (const paths of [["../foreign"], ["/foreign"], [`.mon-stage-${id}`, `.mon-stage-${id}`], ["meetings/all"]]) assert.throws(() => assertRecoveryPreview({ ...p, item: { ...p.item, paths } }));
  assert.throws(() => assertRecoveryListing({ supported: false, root: p.root, items: [p.item] }));
  assert.throws(() => assertRecoveryListing({ supported: true, root: p.root, items: [p.item, p.item] }));
  assert.throws(() => assertRecoveryPreview({ ...p, observedPaths: Number.MAX_SAFE_INTEGER }));
  assert.throws(() => assertRecoveryPreview({ ...p, item: { ...p.item, retained: true } }));
});
test("kept and incomplete recovery are distinct from reclaimed files and published meetings", () => {
  for (const outcome of ["retained", "recovered", "published", "attention"] as const) {
    const r = { source: "scratch" as const, operationId: id, outcome, reason: outcome === "attention" ? "残る" : null };
    assert.doesNotThrow(() => assertRecoveryReceipt(r)); assert.doesNotThrow(() => assertDataSnapshot({ ...ready(), phase: "completed", retryAllowed: false, recoveryReceipt: r }));
    assert.throws(() => assertDataSnapshot({ ...ready(), phase: "completed", retryAllowed: false, recoveryReceipt: { ...r, operationId: "22222222-2222-4222-8222-222222222222" } }));
    if (outcome === "retained") assert.match(recoveryOutcomeLabel(r), /回収はしていません/u);
    if (outcome === "attention") assert.match(recoveryOutcomeLabel(r), /要確認が残っています/u);
  }
});
test("unreadable inventories may explicitly keep but never claim complete inspection", () => {
  assert.doesNotThrow(() => assertRecoveryPreview({ ...preview(), inspectionComplete: false }));
  assert.throws(() => assertRecoveryReceipt({ source: "scratch", operationId: id, outcome: "attention", reason: null }));
});
