import { test } from "node:test";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { TranscriptRecoveryEditor, recoveryContext, type RecoverableDraft } from "../src/transcript-recovery-editor.ts";
import { TranscriptRecoverySaveQueue, type RecoverySaveRequest, type TranscriptRecovery } from "../src/transcript-recovery-save.ts";
import { canonicalPayload, type DraftPayload } from "../src/transcript-state.ts";

const hash = (n: number) => n.toString(16).padStart(64,"0");
const payload = (text?: string): DraftPayload => canonicalPayload({ schema_version: 1,speaker_names: [],utterances: text === undefined ? [] : [{ utterance_id: "u",effective_text: text,speaker_ids: [] }] });
const draft: RecoverableDraft = { id: "draft",meetingId: "meeting",baseAnalysisId: "base",lockVersion: 0,contentHash: hash(100),payload: payload(),updatedAt: 1 };
const buffer = (value = "入力") => ({ utteranceId: "u",text: "原文",speakerIds: [],hadOverride: false,value });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { promise,resolve }; }
function fixture() {
  let actual = structuredClone(draft), record: TranscriptRecovery<DraftPayload> | null = null;
  const calls: { kind: string; req: unknown }[] = [];
  const state = { failJournal: false,lostBody: false,failBody: false,
    journalWait: null as ReturnType<typeof deferred<void>> | null,bodyWait: null as ReturnType<typeof deferred<void>> | null };
  const journal = new TranscriptRecoverySaveQueue<DraftPayload>({
    operationId: randomUUID,normalizePayload: canonicalPayload,
    persist: async (req: RecoverySaveRequest) => {
      calls.push({ kind: "journal",req });
      if(state.journalWait) { const wait = state.journalWait; state.journalWait = null; await wait.promise; }
      if(state.failJournal) throw Error("回復応答不明");
      record = { ...recoveryContext(actual),meetingId: req.meetingId,draftId: req.draftId,baseAnalysisId: req.baseAnalysisId,
        operationId: req.operationId,lockVersion: req.expectedRecoveryLockVersion + 1,contentHash: hash(req.expectedRecoveryLockVersion + 1),updatedAt: 1,
        baseDraftLockVersion: actual.lockVersion,baseDraftContentHash: actual.contentHash,
        currentDraftLockVersion: actual.lockVersion,currentDraftContentHash: actual.contentHash,currentContext: true,matchesDraft: true,
        payload: { schemaVersion: 1,basePayload: actual.payload,buffers: structuredClone(req.buffers) } };
      return record;
    },read: async () => { calls.push({ kind: "readJournal",req: null }); return record; },
  });
  const editor = new TranscriptRecoveryEditor(journal,async (id,version,next) => {
    calls.push({ kind: "body",req: { id,version,payload: structuredClone(next) } });
    if(state.bodyWait) { const wait = state.bodyWait; state.bodyWait = null; await wait.promise; }
    if(state.failBody) throw Error("本文未保存");
    assert.equal(id,actual.id); assert.equal(version,actual.lockVersion);
    actual = { ...actual,payload: structuredClone(next),lockVersion: version + 1,contentHash: hash(101 + version) };
    if(state.lostBody) { state.lostBody = false; throw Error("本文応答喪失"); }
    return structuredClone(actual);
  },async () => { calls.push({ kind: "readBody",req: null }); return structuredClone(actual); });
  editor.load(actual,null);
  return { editor,journal,calls,state,get actual() { return actual; },set actual(value) { actual = value; } };
}
test("body save waits for recovery acknowledgement and rebases remaining input without clearing it", async () => {
  const f = fixture(), wait = deferred<void>(); f.state.journalWait = wait;
  f.journal.update([buffer()]);
  const saving = f.editor.saveDraft(draft.id,0,payload("確定"));
  await Promise.resolve(); await Promise.resolve();
  assert.deepEqual(f.calls.map(c => c.kind),["journal"]);
  assert.equal(f.editor.snapshot().bodyBusy,true);
  wait.resolve(); const saved = await saving;
  assert.deepEqual(f.calls.map(c => c.kind),["journal","body"]);
  assert.equal(saved.lockVersion,1); assert.equal(f.editor.snapshot().context?.lockVersion,1);
  assert.equal(f.editor.snapshot().buffers[0].value,"入力");
  assert.equal(f.editor.snapshot().dirty,true);
  f.journal.update([]); await f.editor.flushRecovery();
  assert.deepEqual(f.calls.map(c => c.kind),["journal","body","journal"]);
  assert.deepEqual((f.calls[2].req as RecoverySaveRequest).buffers,[]);
});
test("failed recovery prevents body publishing and keeps recovery request", async () => {
  const f = fixture(); f.state.failJournal = true; f.journal.update([buffer()]);
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("確定")),/回復応答不明/);
  assert.equal(f.calls.filter(c => c.kind === "body").length,0);
  assert.equal(f.editor.snapshot().bodyPending,false); assert.ok(f.editor.snapshot().pendingRequest);
});
test("typing during body save is retained and recovery flushing is paused", async () => {
  const f = fixture(), wait = deferred<void>(); f.state.bodyWait = wait;
  const saving = f.editor.saveDraft(draft.id,0,payload("旧確定"));
  await Promise.resolve(); await Promise.resolve();
  f.journal.update([buffer("新入力")]);
  await assert.rejects(f.editor.flushRecovery(),/先行する本文保存/);
  wait.resolve(); await saving;
  assert.equal(f.editor.snapshot().buffers[0].value,"新入力");
  assert.equal((f.calls.find(c => c.kind === "body")!.req as { payload: DraftPayload }).payload.utterances[0].effective_text,"旧確定");
  await f.editor.flushRecovery();
  const req = f.calls.find(c => c.kind === "journal")!.req as RecoverySaveRequest;
  assert.equal(req.expectedDraftLockVersion,1); assert.equal(req.buffers[0].value,"新入力");
});
test("lost body acknowledgement is checked read-only and never overwrites newer input", async () => {
  const f = fixture(); f.state.lostBody = true;
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("先行本文")),/応答喪失/);
  f.journal.update([buffer("最新入力")]);
  await assert.rejects(f.editor.flushRecovery(),/先行する本文保存/);
  const before = f.calls.length; const result = await f.editor.recheckBody();
  assert.equal(result.status,"saved"); assert.deepEqual(f.calls.slice(before).map(c => c.kind),["readBody"]);
  assert.equal(f.editor.snapshot().bodyPending,false); assert.equal(f.editor.snapshot().buffers[0].value,"最新入力");
});
test("manual body retry first reads the frozen result without duplicate write", async () => {
  const f = fixture(); f.state.lostBody = true;
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("先行本文")));
  const saved = await f.editor.saveDraft(draft.id,0,payload("最新の別本文"));
  assert.equal(saved.payload.utterances[0].effective_text,"先行本文");
  assert.deepEqual(f.calls.map(c => c.kind),["body","readBody"]);
});
test("unchanged body permits only explicit original retry, not replacing failed request", async () => {
  const f = fixture(); f.state.failBody = true;
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("旧要求")));
  f.state.failBody = false;
  await f.editor.saveDraft(draft.id,0,payload("新要求"));
  assert.deepEqual(f.calls.map(c => c.kind),["body","readBody","body"]);
  assert.deepEqual(f.calls[0].req,f.calls[2].req);
});
test("cancel refuses an already advanced body and does not clear unknown result", async () => {
  const f = fixture(); f.state.lostBody = true;
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("先行本文")));
  await assert.rejects(f.editor.recheckBody(false),/本文は進んでいる/);
  assert.equal(f.editor.snapshot().bodyPending,true); assert.equal(f.calls.filter(c => c.kind === "body").length,1);
});
test("unchanged body read-back clears pending but makes no write", async () => {
  const f = fixture(); f.state.failBody = true;
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("未保存")));
  const result = await f.editor.recheckBody(false);
  assert.equal(result.status,"unchanged"); assert.equal(f.editor.snapshot().bodyPending,false);
  assert.equal(f.calls.filter(c => c.kind === "body").length,1);
});
test("foreign or advanced conflicting bodies cannot resolve pending save or be overwritten", async () => {
  for(const patch of [{ id: "foreign" },{ meetingId: "foreign" },{ baseAnalysisId: "foreign" },{ lockVersion: 2 },{ contentHash: hash(200) }]) {
    const f = fixture(); f.state.failBody = true;
    await assert.rejects(f.editor.saveDraft(draft.id,0,payload("保持")));
    f.actual = { ...f.actual,...patch };
    await assert.rejects(f.editor.recheckBody());
    await assert.rejects(f.editor.saveDraft(draft.id,0,payload("別入力")));
    assert.equal(f.editor.snapshot().bodyPending,true); assert.equal(f.calls.filter(c => c.kind === "body").length,1);
  }
});
test("dispose during body save rejects late acknowledgement and prevents follow-up recovery", async () => {
  const f = fixture(), wait = deferred<void>(); f.state.bodyWait = wait;
  const saving = f.editor.saveDraft(draft.id,0,payload("確定"));
  await Promise.resolve(); await Promise.resolve(); f.editor.dispose(); wait.resolve();
  await assert.rejects(saving,/終了/); await assert.rejects(f.editor.flushRecovery(),/終了/);
  assert.equal(f.calls.filter(c => c.kind === "journal").length,0);
});
test("unsafe next body version is refused before any body request is frozen or sent", async () => {
  const f = fixture(); f.actual = { ...f.actual,lockVersion: Number.MAX_SAFE_INTEGER };
  f.editor.load(f.actual,null);
  await assert.rejects(f.editor.saveDraft(draft.id,Number.MAX_SAFE_INTEGER,payload("保持")),/安全な範囲/);
  assert.equal(f.calls.length,0); assert.equal(f.editor.snapshot().bodyPending,false);
});
