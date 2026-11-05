import test from "node:test";
import assert from "node:assert/strict";
import { deletionAvailability, deletionReviewReason, permitsDeletionRecovery } from "../src/deletion-review-reason.ts";
import { assertDataAvailability } from "../src/data-operation-state.ts";
import type { DataOperation } from "../src/data-operation.ts";

test("deletion explains the actual shared recording/AI blocker rather than platform support", () => {
  const operation = { connected: true, availability: { available: false, reason: "AI処理中" } } as DataOperation;
  const state = { native: true, supported: true, locked: false, protectedNow: false, sharedAvailable: false };
  assert.equal(deletionReviewReason(operation, state), "AI処理中のため削除できません。");
  operation.availability = { available: false, reason: "録音・音声保存中" };
  assert.equal(deletionReviewReason(operation, state), "録音・音声保存中のため削除できません。");
});
test("unverified state, restart, and pending recovery each retain an actionable cause", () => {
  const state = { native: true, supported: true, locked: false, protectedNow: false, sharedAvailable: false };
  const operation = { connected: false, error: "所有台帳の確認失敗" } as DataOperation;
  assert.match(deletionReviewReason(operation, state), /所有台帳の確認失敗/);
  operation.connected = true;
  operation.snapshot = { restartRequired: true } as NonNullable<DataOperation["snapshot"]>;
  assert.match(deletionReviewReason(operation, state), /再起動/);
  operation.snapshot = null;
  operation.deletionListing = { writeBlocked: true } as NonNullable<DataOperation["deletionListing"]>;
  assert.match(deletionReviewReason(operation, state), /未完了.*設定画面/);
});

test("a pending model receipt and a failed read are explained without claiming Windows lacks deletion", () => {
  const state = {native:true,supported:true,locked:false,protectedNow:false,sharedAvailable:true};
  const operation = {connected:true,snapshot:{modelDeletionPreview:{pending:true}}} as DataOperation;
  assert.match(deletionReviewReason(operation,state), /未完了/);
  operation.connected = false; operation.error = "権限の状態を取得できません";
  assert.match(deletionReviewReason(operation,state), /権限/);
  operation.connected = true; operation.snapshot = null; operation.error = "";
  assert.doesNotMatch(deletionReviewReason(operation,state), /このアプリでは削除機能/);
});

test("deletion admission uses its native lane while retaining real blockers and rejects incomplete replies", () => {
  assert.deepEqual(deletionAvailability({available:false,reason:"録音通知履歴の確認・更新中",reasonCode:"operation_active",deletionAvailable:true,deletionReason:null,deletionReasonCode:null}), {available:true,reason:null,reasonCode:null});
  for (const reason of ["AI処理中", "モデル準備中", "録音・音声保存中", "保存先切替中"]) {
    const availability = {available:false,reason:"録音通知履歴の確認・更新中",reasonCode:"operation_active" as const,deletionAvailable:false,deletionReason:reason,deletionReasonCode:"operation_active" as const};
    const admission = deletionAvailability(availability);
    assert.equal(admission.available, false);
    const operation = {connected:true,availability} as DataOperation;
    assert.equal(deletionReviewReason(operation,{native:true,supported:true,locked:false,protectedNow:false,sharedAvailable:admission.available,availabilityReason:admission.reason}), `${reason}のため削除できません。`);
  }
  assert.throws(() => assertDataAvailability({available:true,reason:null} as never));
  assert.throws(() => assertDataAvailability({available:false,reason:"AI処理中"} as never));
  assert.equal(deletionAvailability(null).available, false);
});

test("recovery admission uses its own fixed code independently of translated copy", () => {
  const code = "relocation_restart_required" as const;
  const value = {available:false,reason:"保存先の変更後に再起動してください",reasonCode:code,deletionAvailable:false,deletionReason:"再起動待ち",deletionReasonCode:code};
  assertDataAvailability(value);
  assert.equal(deletionAvailability(value).reasonCode, code);
  assert.equal(deletionAvailability({...value,deletionReasonCode:null} as unknown as Parameters<typeof deletionAvailability>[0]).reasonCode, null);
  assert.throws(() => assertDataAvailability({...value,deletionReasonCode:null}));
  assert.equal(deletionAvailability({...value,deletionReasonCode:"operation_active"}).reasonCode, "operation_active");
});

test("identical Japanese text cannot authorize pending deletion recovery with a different reason code", () => {
  const reason = "保存先の切替後は終了して再起動が必要です";
  assert.equal(permitsDeletionRecovery({reasonCode:"relocation_restart_required",reason} as Parameters<typeof permitsDeletionRecovery>[0], true), true);
  for (const reasonCode of [undefined, null, "operation_active", "recording_active"] as const) {
    assert.equal(permitsDeletionRecovery({reasonCode,reason} as Parameters<typeof permitsDeletionRecovery>[0], true), false);
  }
  assert.equal(permitsDeletionRecovery({reasonCode:"relocation_restart_required"}, false), false);
  assert.equal(permitsDeletionRecovery(null, true), false);
});
