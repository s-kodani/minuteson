import { test } from "node:test";
import assert from "node:assert/strict";
import { waitForDataStop } from "../src/data-stop.ts";
import type { DataSnapshot } from "../src/data-operation-state.ts";
const token = "11111111-1111-4111-8111-111111111111";
const snapshot = (busy: boolean): DataSnapshot => ({ revision: 1, token, kind: "backup", phase: busy ? "cancelling" : "cancelled", busy, backupPreview: null, backupReceipt: null, restorePreview: null, restoreReceipt: null, relocationPreview: null, relocationReceipt: null, restartRequired: false, attentionRequired: false, retryAllowed: false, errorMessage: null });
test("data stop observes same token until actual terminal state", async () => {
  let reads = 0, waits = 0;
  const result = await waitForDataStop(token, "backup", async () => { reads++; return snapshot(reads < 3); }, async () => { waits++; });
  assert.equal(result.phase, "cancelled"); assert.equal(reads, 3); assert.equal(waits, 2);
});
test("changed identity and lost read fail without a cancellation replay", async () => {
  await assert.rejects(waitForDataStop(token, "backup", async () => ({ ...snapshot(true), token: "21111111-1111-4111-8111-111111111111" })), /停止対象/u);
  await assert.rejects(waitForDataStop(token, "backup", async () => ({ ...snapshot(true), kind: "restore" })), /停止対象/u);
  await assert.rejects(waitForDataStop(token, "backup", async () => { throw Error("読戻し喪失"); }), /読戻し喪失/u);
});
test("bounded stop observation never fabricates successful cancellation", async () => {
  let time = 0, waits = 0;
  await assert.rejects(waitForDataStop(token, "backup", async () => snapshot(true), async () => { time += 60_000; waits++; }, () => time), /まだ完了していません/u);
  assert.equal(waits, 2);
});
test("a publication winning cancellation is retained as completed not rollback", async () => {
  const completed: DataSnapshot = { ...snapshot(false), phase: "completed", backupReceipt: { archiveId: token, meetingCount: 1, assetCount: 1, deletedAudioMeetingCount: 0, deletedAudioAssetCount: 0, size: 128, sha256: "a".repeat(64) } };
  const result = await waitForDataStop(token, "backup", async () => completed);
  assert.equal(result, completed); assert.equal(result.phase, "completed");
});
