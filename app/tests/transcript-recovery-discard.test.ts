import { test } from "node:test";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { TranscriptRecoveryDiscard, type RecoveryDiscardRequest } from "../src/transcript-recovery-discard.ts";
import type { TranscriptRecovery } from "../src/transcript-recovery-save.ts";
const h = (n: number) => n.toString(16).padStart(64,"0");
const old: TranscriptRecovery<{ text: string }> = { meetingId: "meeting",draftId: "draft",baseAnalysisId: "base",operationId: randomUUID(),
  contentHash: h(1),lockVersion: 1,updatedAt: 1,baseDraftLockVersion: 0,baseDraftContentHash: h(100),
  currentDraftLockVersion: 2,currentDraftContentHash: h(102),currentContext: false,matchesDraft: false,
  payload: { schemaVersion: 1,basePayload: { text: "旧本文" },buffers: [{ utteranceId: "u",text: "旧本文",value: "保持する未確定入力",speakerIds: [],hadOverride: false }] } };
function result(req: RecoveryDiscardRequest) { return { ...structuredClone(old),operationId: req.operationId,lockVersion: req.expectedRecoveryLockVersion + 1,contentHash: h(2),
  baseDraftLockVersion: req.expectedDraftLockVersion,baseDraftContentHash: req.expectedDraftContentHash,
  payload: { schemaVersion: 2,basePayload: { text: "現在本文" },buffers: [],discardedFrom: { contentHash: req.expectedRecoveryContentHash,lockVersion: req.expectedRecoveryLockVersion } } }; }
function fixture() {
  const calls: RecoveryDiscardRequest[] = []; let actual = structuredClone(old), reads = 0;
  const state = { fail: false,lost: false,patch: {} };
  const controller = new TranscriptRecoveryDiscard<{ text: string }>(async req => {
    calls.push(req); if(state.fail) throw Error("破棄失敗"); actual = { ...result(req),...state.patch };
    if(state.lost) throw Error("応答喪失"); return actual;
  },async () => { reads++; return actual; },randomUUID);
  return { controller,calls,state,get reads() { return reads; },set actual(value) { actual = value; } };
}
test("explicit stale preview discard uses current draft stamp and exact recovery proof", async () => {
  const f = fixture(), saved = await f.controller.start(old);
  assert.equal(f.calls[0].expectedDraftLockVersion,2); assert.equal(f.calls[0].expectedDraftContentHash,h(102));
  assert.equal(f.calls[0].expectedRecoveryContentHash,old.contentHash); assert.equal(saved.payload.buffers.length,0);
  assert.equal(f.controller.snapshot().pending,null);
});
test("name-only consent and lost-ack read-back clear names without another deletion", async () => {
  const f = fixture(); f.state.lost = true;
  const preview = structuredClone(old);
  preview.payload.schemaVersion = 3; preview.payload.buffers = [];
  preview.payload.nameBuffers = [{ speakerId: "s",name: null,hadOverride: false,value: "保持する名" }];
  await assert.rejects(f.controller.start(preview),/応答喪失/);
  preview.payload.nameBuffers[0].value = "後から変更";
  assert.equal(f.controller.snapshot().pending?.preview.payload.nameBuffers?.[0].value,"保持する名");
  assert.equal((await f.controller.recheck()).status,"discarded");
  assert.equal(f.calls.length,1); assert.equal(f.reads,1);
});
test("discard proof that still contains names never counts as successful deletion", async () => {
  const f = fixture();
  f.state.patch = { payload: { schemaVersion: 2,basePayload: old.payload.basePayload,buffers: [],
    nameBuffers: [{ speakerId: "s",name: null,hadOverride: false,value: "残っている名" }],
    discardedFrom: { contentHash: old.contentHash,lockVersion: old.lockVersion } } };
  await assert.rejects(f.controller.start(old),/一致しません/);
  await assert.rejects(f.controller.recheck(),/別の回復記録/);
  assert.ok(f.controller.snapshot().pending); assert.equal(f.calls.length,1);
});
test("lost discard ack keeps original consent through input mutation and retry", async () => {
  const f = fixture(); f.state.lost = true; const preview = structuredClone(old);
  await assert.rejects(f.controller.start(preview),/応答喪失/);
  preview.payload.buffers[0].value = "後から変更";
  assert.equal(f.controller.snapshot().pending?.preview.payload.buffers[0].value,"保持する未確定入力");
  await assert.rejects(f.controller.start(old),/先行する破棄結果/);
  f.state.lost = false; await f.controller.retry(); assert.equal(f.calls[0],f.calls[1]);
  assert.throws(() => { f.calls[0].expectedRecoveryLockVersion = 10; },TypeError);
});
test("read-only lost discard result check accepts proof without new deletion", async () => {
  const f = fixture(); f.state.lost = true; await assert.rejects(f.controller.start(old));
  assert.equal((await f.controller.recheck()).status,"discarded"); assert.equal(f.calls.length,1); assert.equal(f.reads,1);
  assert.equal(f.controller.snapshot().pending,null);
});
test("unchanged discard head retains consent and never writes on result check", async () => {
  const f = fixture(); f.state.fail = true; await assert.rejects(f.controller.start(old));
  assert.equal((await f.controller.recheck()).status,"unchanged"); assert.equal(f.calls.length,1);
  assert.ok(f.controller.snapshot().pending);
});
test("mismatched discard proof, owner, operation and baseline remain unknown", async () => {
  for(const patch of [{ meetingId: "foreign" },{ draftId: "foreign" },{ baseAnalysisId: "foreign" },{ operationId: randomUUID() },{ lockVersion: 7 },{ baseDraftContentHash: h(200) },{ contentHash: "bad" },
    { payload: { ...old.payload,schemaVersion: 2,buffers: [],discardedFrom: { contentHash: h(200),lockVersion: 1 } } }]) {
    const f = fixture(); f.state.patch = patch; await assert.rejects(f.controller.start(old),/一致しません/);
    await assert.rejects(f.controller.recheck(),/別の回復記録/);
    assert.ok(f.controller.snapshot().pending); assert.equal(f.calls.length,1);
  }
});
test("dispose forbids follow-up discard and result checks", async () => {
  const f = fixture(); f.state.fail = true; await assert.rejects(f.controller.start(old)); f.controller.dispose();
  await assert.rejects(f.controller.retry(),/終了/); await assert.rejects(f.controller.recheck(),/終了/); assert.equal(f.calls.length,1);
});
