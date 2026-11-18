import { test } from "node:test";
import assert from "node:assert/strict";
import { assertCloseContext, assertCloseTargets, closeMessage, closeTarget, dataNeedsConsent, importNeedsConsent, waitForImportStop, waitForRecordingSave, waitForRecordingSaves, type CloseContext } from "../src/application-close.ts";
import type { RecoverySnapshot } from "../src/recording-recovery-state.ts";
const token = "11111111-1111-4111-8111-111111111111";
const other = "21111111-1111-4111-8111-111111111111";
const context = (): CloseContext => ({ recording: { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null }, importing: { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, fileName: null, info: null, sourceBytes: null, requiredBytes: null, availableBytes: null, completed: 0, saved: null, errorMessage: null }, data: { revision: 0, token: null, kind: null, phase: "idle", busy: false, backupPreview: null, restorePreview: null, backupReceipt: null, restoreReceipt: null, relocationPreview: null, relocationReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: false }, activity: { analysisBusy: false, modelPreparationBusy: false } });

const savingBatch = (): RecoverySnapshot => ({runId:"run",revision:3,origin:"startup",targetSessionId:null,phase:"running",error:null,items:[
  {sessionId:token,meetingId:token,meetingTitle:"会議",phase:"running",startedAtMs:10,error:null},
  {sessionId:other,meetingId:other,meetingTitle:"次の会議",phase:"queued",startedAtMs:null,error:null},
]});
test("startup saving waits for every target, and scanning transfers its discovered inventory", async () => {
  const batch=savingBatch();let reads=0,waits=0;
  const done={...batch,revision:5,phase:"completed" as const,items:batch.items.map(i=>({...i,phase:"succeeded" as const}))};
  const result=await waitForRecordingSaves({...batch,phase:"scanning",items:[]},async()=>++reads<3?batch:done,async()=>{waits++;});
  assert.equal(result,done);assert.equal(waits,2);
});
test("saving failures, stale replies, new targets, changed run and lost reads keep the app open", async () => {
  const batch=savingBatch();
  await assert.rejects(waitForRecordingSaves(batch,async()=>({...batch,revision:2})),/古い/);
  await assert.rejects(waitForRecordingSaves(batch,async()=>({...batch,runId:"new"})),/変更/);
  await assert.rejects(waitForRecordingSaves(batch,async()=>({...batch,items:batch.items.slice(0,1)})),/変更/);
  await assert.rejects(waitForRecordingSaves(batch,async()=>({...batch,phase:"completed",items:batch.items.map(i=>({...i,phase:"failed",error:"保存失敗"}))})),/保存失敗/);
  await assert.rejects(waitForRecordingSaves(batch,async()=>{throw Error("read lost");}),/read lost/);
  let clock=0;
  await assert.rejects(waitForRecordingSaves(batch,async()=>batch,async()=>{clock+=60_000;},()=>clock),/まだ完了/);
});
test("a successful retry checks its target without treating an older retained failure as current", async () => {
  const batch=savingBatch();
  const retry:RecoverySnapshot={...batch,origin:"retry",targetSessionId:token,items:batch.items.map(i=>({...i,phase:i.sessionId===token?"running":"failed",error:i.sessionId===token?null:"以前の失敗"}))};
  const done:RecoverySnapshot={...retry,phase:"completed",revision:4,items:retry.items.map(i=>i.sessionId===token?{...i,phase:"succeeded"}:i)};
  assert.equal(await waitForRecordingSaves(retry,async()=>done),done);
  const c={...context(),recovery:done};
  assert.throws(()=>assertCloseTargets(c,{...c,recovery:{...done,revision:5}}),/変更/);
});
test("close reviews validate native states instead of assuming unknown states are idle", () => {
  const c = context(); assertCloseContext(c);
  assert.throws(() => assertCloseContext({ ...c, recording: { ...c.recording, phase: "saving" } }), /録音状態/u);
  assert.throws(() => assertCloseContext({ ...c, importing: { ...c.importing, busy: true } }), /取り込み状態/u);
  assert.throws(() => assertCloseContext({ ...c, importing: { ...c.importing, phase: "future" } }), /取り込み状態/u);
  assert.throws(() => assertCloseContext({ ...c, activity: { analysisBusy: null as unknown as boolean, modelPreparationBusy: false } }), /終了対象/u);
});
test("quiet close does not require destructive consent and ready imports do", () => {
  const c = context(); assert.equal(dataNeedsConsent(c.data), false); assert.equal(importNeedsConsent(c.importing), false);
  assert.equal(importNeedsConsent({ ...c.importing, token, phase: "ready" }), true);
  assert.equal(dataNeedsConsent({ ...c.data, token, kind: "backup", retryAllowed: true }), true);
});
test("closing rejects new tokens, operation kinds, ready revisions and unreviewed recording", () => {
  const c = context(); assertCloseTargets(c, structuredClone(c));
  assert.throws(() => assertCloseTargets(c, { ...c, importing: { ...c.importing, token: other } }), /変更/u);
  assert.throws(() => assertCloseTargets(c, { ...c, data: { ...c.data, token: other, kind: "backup" } }), /変更/u);
  assert.throws(() => assertCloseTargets(c, { ...c, importing: { ...c.importing, revision: 1 } }), /変更/u);
  assert.throws(() => assertCloseTargets(c, { ...c, recording: { ...c.recording, phase: "error", sessionId: other, meetingId: other } }), /変更/u);
});
test("busy progress and a winning publication retain the reviewed operation", () => {
  const c = context(); c.data = { ...c.data, token, kind: "backup", busy: true, phase: "publishing" };
  assertCloseTargets(c, { ...c, data: { ...c.data, revision: 2 } });
  const saved = { ...c.data, revision: 3, busy: false, phase: "completed", backupReceipt: { archiveId: token, meetingCount: 1, assetCount: 0, deletedAudioMeetingCount: 0, deletedAudioAssetCount: 0, size: 12, sha256: "a".repeat(64) } };
  assertCloseTargets(c, { ...c, data: saved });
  assert.equal(saved.backupReceipt.archiveId, token);
});
test("closing preserves deletion loss warnings and relocation copies", () => {
  const c = context();
  for (const kind of ["deletion", "audio_deletion"] as const) {
    const message = closeMessage({ ...c, data: { ...c.data, kind, token, retryAllowed: true } });
    assert.match(message, /既に消した/u); assert.match(message, /復元しません/u); assert.match(message, /未完了記録/u);
  }
  assert.match(closeMessage({ ...c, data: { ...c.data, kind: "relocation", token, retryAllowed: true } }), /旧保存先と途中コピー/u);
});
test("import cancellation waits for actual terminal state and keeps winning publication", async () => {
  const c = context(); let reads = 0, waits = 0;
  const result = await waitForImportStop(token, async () => ({ ...c.importing, token, revision: ++reads, busy: reads < 3, phase: reads < 3 ? "cancelling" : "saved", saved: reads < 3 ? null : { id: token, meetingId: token, state: "saved" } }), async () => { waits++; });
  assert.equal(waits, 2); assert.equal(result.phase, "saved"); assert.equal(result.saved?.meetingId, token);
});
test("import lost reads, changed targets and time limits never fabricate cancellation", async () => {
  const c = context();
  await assert.rejects(waitForImportStop(token, async () => ({ ...c.importing, token: other })), /変更/u);
  await assert.rejects(waitForImportStop(token, async () => { throw Error("read lost"); }), /read lost/u);
  let clock = 0;
  await assert.rejects(waitForImportStop(token, async () => ({ ...c.importing, token, busy: true, phase: "cancelling" }), async () => { clock += 60_000; }, () => clock), /まだ完了/u);
});
test("recording save observation never sends stop and requires real idle", async () => {
  const c = context(), saving = { ...c.recording, revision: 1, sessionId: token, meetingId: token, phase: "saving" as const };
  let reads = 0, waits = 0;
  await waitForRecordingSave(saving, async () => ++reads < 3 ? saving : { ...c.recording, revision: 2 }, async () => { waits++; });
  assert.equal(waits, 2);
});
test("recording failures, stale states, new sessions and time limits retain the open app", async () => {
  const c = context(), saving = { ...c.recording, revision: 1, sessionId: token, meetingId: token, phase: "saving" as const };
  await assert.rejects(waitForRecordingSave(saving, async () => ({ ...saving, phase: "error", errorMessage: "save failed" })), /save failed/u);
  await assert.rejects(waitForRecordingSave(saving, async () => c.recording), /古い/u);
  await assert.rejects(waitForRecordingSave(saving, async () => ({ ...saving, sessionId: other })), /変更/u);
  let clock = 0;
  await assert.rejects(waitForRecordingSave(saving, async () => saving, async () => { clock += 60_000; }, () => clock), /まだ完了/u);
});


test("recovery activity is validated and a newly started recovery invalidates old exit consent", () => {
  const idle = context();
  const recovering = {...idle, activity:{...idle.activity, recordingRecoveryBusy:true}};
  assertCloseContext(recovering);
  assert.equal(closeTarget(recovering), "録音の保存");
  assert.throws(()=>assertCloseTargets(idle,recovering), /保存/);
  assert.throws(()=>assertCloseContext({...idle,activity:{...idle.activity,recordingRecoveryBusy:"unknown" as unknown as boolean}}), /保存/);
  assertCloseTargets(recovering,idle);
});
