import test from "node:test";
import assert from "node:assert/strict";
import { deletionAvailability, deletionReviewReason } from "../src/deletion-review-reason.ts";
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

test("deletion admission uses its native lane while retaining real blockers and old-build compatibility", () => {
  assert.deepEqual(deletionAvailability({available:false,reason:"録音通知履歴の確認・更新中",deletionAvailable:true,deletionReason:null}), {available:true,reason:null});
  for (const reason of ["AI処理中", "モデル準備中", "録音・音声保存中", "保存先切替中"]) {
    const availability = {available:false,reason:"録音通知履歴の確認・更新中",deletionAvailable:false,deletionReason:reason};
    const admission = deletionAvailability(availability);
    assert.equal(admission.available, false);
    const operation = {connected:true,availability} as DataOperation;
    assert.equal(deletionReviewReason(operation,{native:true,supported:true,locked:false,protectedNow:false,sharedAvailable:admission.available,availabilityReason:admission.reason}), `${reason}のため削除できません。`);
  }
  assert.deepEqual(deletionAvailability({available:true,reason:null}), {available:true,reason:null});
  assert.deepEqual(deletionAvailability({available:false,reason:"AI処理中"}), {available:false,reason:"AI処理中"});
  assert.equal(deletionAvailability(null).available, false);
});
