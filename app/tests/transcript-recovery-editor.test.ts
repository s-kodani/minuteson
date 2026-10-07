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
const nameBuffer = (value = "未確定名") => ({ speakerId: "s",name: null,hadOverride: false,value });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { promise,resolve }; }
function fixture() {
  let actual = structuredClone(draft), record: TranscriptRecovery<DraftPayload> | null = null;
  const calls: { kind: string; req: unknown }[] = [];
  const state = { failJournal: false,lostBody: false,failBody: false,failReadBody: false,
    readBodyWait: null as ReturnType<typeof deferred<void>> | null,
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
        payload: { schemaVersion: req.nameBuffers?.length ? 3 : 1,basePayload: actual.payload,buffers: structuredClone(req.buffers),
          ...(req.nameBuffers?.length ? { nameBuffers: structuredClone(req.nameBuffers) } : {}) } };
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
  },async () => {
    calls.push({ kind: "readBody",req: null });
    if(state.readBodyWait) { const wait = state.readBodyWait; state.readBodyWait = null; await wait.promise; }
    if(state.failReadBody) throw Error("本文読込失敗");
    return structuredClone(actual);
  });
  editor.load(actual,null);
  return { editor,journal,calls,state,get actual() { return actual; },set actual(value) { actual = value; },
    get record() { return record; },set record(value) { record = value; } };
}
function advanceExternally(f: ReturnType<typeof fixture>) {
  f.actual = { ...f.actual,lockVersion: 2,contentHash: hash(102),payload: payload("別操作の本文") };
  if(f.record) f.record = { ...f.record,currentDraftLockVersion: f.actual.lockVersion,currentDraftContentHash: f.actual.contentHash,matchesDraft: false };
}
test("advanced draft comparison exposes latest body and the frozen unresolved request without writing or clearing input", async () => {
  const f = fixture(); f.journal.update([buffer()],[nameBuffer()]); f.state.failBody = true;
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("先行する手元要求"))); advanceExternally(f);
  const before = f.editor.snapshot(), callCount = f.calls.length;
  const review = await f.editor.inspectAdvancedDraft();
  assert.deepEqual(f.calls.slice(callCount).map(call => call.kind),["readBody","readJournal"]);
  assert.deepEqual(f.editor.snapshot(),before); assert.equal(review.latest.lockVersion,2);
  assert.equal(review.pendingPayload?.utterances[0].effective_text,"先行する手元要求");
  assert.deepEqual(review.recovery.nameBuffers,[nameBuffer()]);
  assert.throws(() => { review.latest.payload.utterances[0].effective_text = "改変"; },TypeError);
});
test("explicit advanced draft acceptance rereads body and journal, keeps selected input and makes no publish or recovery write", async () => {
  const f = fixture(); f.journal.update([buffer()],[nameBuffer()]); f.state.failBody = true;
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("元の要求"))); advanceExternally(f);
  const review = await f.editor.inspectAdvancedDraft(), callCount = f.calls.length;
  const selectedText = { ...buffer("選んだ入力"),text: "別操作の本文",hadOverride: true };
  const latest = await f.editor.acceptAdvancedDraftReview(review,[selectedText],[nameBuffer("選んだ名前")]);
  assert.equal(latest.lockVersion,2);
  assert.deepEqual(f.calls.slice(callCount).map(call => call.kind),["readBody","readJournal"]);
  assert.equal(f.editor.snapshot().bodyPending,false); assert.equal(f.editor.snapshot().context?.lockVersion,2);
  assert.deepEqual(f.editor.snapshot().buffers,[selectedText]); assert.deepEqual(f.editor.snapshot().nameBuffers,[nameBuffer("選んだ名前")]);
  assert.equal(f.actual.payload.utterances[0].effective_text,"別操作の本文");
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("古い版の再送")),/一致/);
  assert.equal(f.calls.length,callCount + 2);
  f.state.failBody = false;
  await f.editor.saveDraft(draft.id,2,payload("明示的な新要求"));
  assert.equal(f.actual.lockVersion,3);
  const lastJournal = f.calls.filter(call => call.kind === "journal").at(-1)!.req as RecoverySaveRequest;
  assert.equal(lastJournal.expectedDraftLockVersion,2);
  assert.equal(f.calls.filter(call => call.kind === "body").length,2);
});
test("recovery-only pending conflict can be reviewed and retired only against a newer same draft", async () => {
  const f = fixture(); f.state.failJournal = true; f.journal.update([buffer()],[nameBuffer()]);
  await assert.rejects(f.editor.saveDraft(draft.id,0,payload("未送信本文")));
  const original = f.editor.snapshot().pendingRequest;
  assert.equal(f.editor.snapshot().bodyPending,false); advanceExternally(f);
  const review = await f.editor.inspectAdvancedDraft();
  assert.equal(f.editor.snapshot().pendingRequest,original);
  await f.editor.acceptAdvancedDraftReview(review,[buffer()],[nameBuffer()]);
  assert.equal(f.editor.snapshot().pendingRequest,null); assert.equal(f.editor.snapshot().bodyPending,false);
  assert.equal(f.calls.filter(call => call.kind === "body").length,0);
  assert.equal(f.calls.filter(call => call.kind === "journal").length,1);
});
test("advanced review rejects foreign, unchanged, invalid and backward bodies without releasing pending save", async () => {
  for(const patch of [{ id: "other" },{ meetingId: "other" },{ baseAnalysisId: "other" },{ lockVersion: 0 },{ lockVersion: -1 },{ lockVersion: Number.MAX_SAFE_INTEGER + 1 },{ contentHash: "invalid" }]) {
    const f = fixture(); f.state.failBody = true;
    await assert.rejects(f.editor.saveDraft(draft.id,0,payload("保持"))); advanceExternally(f); f.actual = { ...f.actual,...patch };
    await assert.rejects(f.editor.inspectAdvancedDraft());
    assert.equal(f.editor.snapshot().bodyPending,true); assert.equal(f.editor.snapshot().context?.lockVersion,0);
    assert.equal(f.calls.filter(call => call.kind === "readJournal").length,0);
  }
});
test("body changing after comparison rejects rebase and retains the original body request and both input types", async () => {
  for(const patch of [{ id: "other" },{ lockVersion: 3 },{ contentHash: hash(200) },{ payload: payload("さらに変更") }]) {
    const f = fixture(); f.journal.update([buffer()],[nameBuffer()]); f.state.failBody = true;
    await assert.rejects(f.editor.saveDraft(draft.id,0,payload("元要求"))); advanceExternally(f);
    const review = await f.editor.inspectAdvancedDraft(), before = f.editor.snapshot(); f.actual = { ...f.actual,...patch };
    await assert.rejects(f.editor.acceptAdvancedDraftReview(review,[],[]),/比較後/);
    assert.deepEqual(f.editor.snapshot(),before); assert.equal(f.calls.filter(call => call.kind === "body").length,1);
  }
});
test("changed journal, newer local input, forged and consumed tokens cannot release a compared body request", async () => {
  for(const change of ["journal","typing","forged"]) {
    const f = fixture(); f.journal.update([buffer()],[nameBuffer()]); f.state.failBody = true;
    await assert.rejects(f.editor.saveDraft(draft.id,0,payload("元要求"))); advanceExternally(f);
    const review = await f.editor.inspectAdvancedDraft();
    if(change === "journal") f.record = { ...f.record!,operationId: randomUUID() };
    if(change === "typing") f.journal.update([buffer("新入力")],[nameBuffer("新名")]);
    await assert.rejects(f.editor.acceptAdvancedDraftReview(change === "forged" ? structuredClone(review) : review,[],[]));
    assert.equal(f.editor.snapshot().bodyPending,true); assert.equal(f.editor.snapshot().context?.lockVersion,0);
    assert.equal(f.editor.snapshot().nameBuffers[0].value,change === "typing" ? "新名" : "未確定名");
  }
  const f = fixture(); advanceExternally(f); const review = await f.editor.inspectAdvancedDraft();
  await f.editor.acceptAdvancedDraftReview(review,[],[]);
  await assert.rejects(f.editor.acceptAdvancedDraftReview(review,[],[]));
});
test("failed read and disposal while accepting a body comparison leave the old base and input intact", async () => {
  for(const dispose of [false,true]) {
    const f = fixture(); f.journal.update([buffer()],[nameBuffer()]); advanceExternally(f);
    const review = await f.editor.inspectAdvancedDraft();
    if(dispose) {
      const wait = deferred<void>(); f.state.readBodyWait = wait;
      const accepting = f.editor.acceptAdvancedDraftReview(review,[],[]); f.editor.dispose(); wait.resolve();
      await assert.rejects(accepting,/終了/);
    } else { f.state.failReadBody = true; await assert.rejects(f.editor.acceptAdvancedDraftReview(review,[],[]),/読込失敗/); }
    assert.equal(f.editor.snapshot().context?.lockVersion,0); assert.equal(f.editor.snapshot().buffers[0].value,"入力");
    assert.equal(f.calls.filter(call => call.kind === "journal" || call.kind === "body").length,0);
  }
});
test("body inspection/acceptance waits for active work and freezes selected input across its final reads", async () => {
  const f = fixture(), wait = deferred<void>(); f.state.bodyWait = wait;
  const saving = f.editor.saveDraft(draft.id,0,payload("元"));
  for(let i = 0; i < 20 && !f.calls.some(call => call.kind === "body"); i++) await new Promise(resolve => setImmediate(resolve));
  await assert.rejects(f.editor.inspectAdvancedDraft()); wait.resolve(); await saving;
  advanceExternally(f); const review = await f.editor.inspectAdvancedDraft(), readWait = deferred<void>();
  f.state.readBodyWait = readWait;
  const selected = buffer("選択した値"), selectedName = nameBuffer("選択した名");
  const accepting = f.editor.acceptAdvancedDraftReview(review,[selected],[selectedName]);
  selected.value = "外部変更"; selectedName.value = "外部変更";
  await assert.rejects(f.editor.inspectAdvancedDraft()); readWait.resolve(); await accepting;
  assert.equal(f.editor.snapshot().buffers[0].value,"選択した値"); assert.equal(f.editor.snapshot().nameBuffers[0].value,"選択した名");
});
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
test("production load reviews named and mixed records, then explicit read-only resume restores both arrays", async () => {
  for(const mixed of [false,true]) {
    const f = fixture();
    const record: TranscriptRecovery<DraftPayload> = {
      meetingId: draft.meetingId,draftId: draft.id,baseAnalysisId: draft.baseAnalysisId,
      operationId: randomUUID(),lockVersion: 1,contentHash: hash(1),updatedAt: 1,
      baseDraftLockVersion: draft.lockVersion,baseDraftContentHash: draft.contentHash,
      currentDraftLockVersion: draft.lockVersion,currentDraftContentHash: draft.contentHash,
      currentContext: true,matchesDraft: true,
      payload: { schemaVersion: 3,basePayload: draft.payload,buffers: mixed ? [buffer("保存済み本文")] : [],
        nameBuffers: [{ speakerId: "s",name: null,hadOverride: false,value: "保存済みの未確定名" }] },
    };
    f.record = record; f.editor.load(draft,record);
    assert.deepEqual(f.editor.snapshot().buffers,[]); assert.deepEqual(f.editor.snapshot().nameBuffers,[]);
    assert.deepEqual(f.editor.snapshot().review,record);
    assert.equal(f.calls.length,0);
    await f.editor.resume();
    assert.deepEqual(f.calls.map(call => call.kind),["readJournal"]);
    assert.deepEqual(f.editor.snapshot().buffers,record.payload.buffers);
    assert.deepEqual(f.editor.snapshot().nameBuffers,record.payload.nameBuffers);
    assert.equal(f.editor.snapshot().review,null); assert.equal(f.editor.snapshot().dirty,false);
    assert.deepEqual(f.actual,draft);
  }
});
test("mixed recovery is acknowledged before publishing only the chosen name, then preserves text and a newer name", async () => {
  const f = fixture(), wait = deferred<void>(); f.state.bodyWait = wait;
  f.journal.update([buffer()],[nameBuffer("確定する名")]);
  const selected = canonicalPayload({ ...draft.payload,speaker_names: [{ speaker_id: "s",display_name: "確定する名" }] });
  const saving = f.editor.saveDraft(draft.id,0,selected);
  for(let attempt = 0; attempt < 20 && !f.calls.some(call => call.kind === "body"); attempt++) await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(f.calls.map(call => call.kind),["journal","body"]);
  assert.deepEqual((f.calls[0].req as RecoverySaveRequest).nameBuffers,[nameBuffer("確定する名")]);
  f.journal.update([buffer("新本文")],[nameBuffer("さらに新しい名")]);
  wait.resolve(); await saving; await f.editor.flushRecovery();
  assert.deepEqual(f.actual.payload,selected);
  assert.deepEqual(f.editor.snapshot().buffers,[buffer("新本文")]);
  assert.deepEqual(f.editor.snapshot().nameBuffers,[nameBuffer("さらに新しい名")]);
  assert.equal((f.calls.at(-1)!.req as RecoverySaveRequest).expectedDraftLockVersion,1);
});
test("clearing only a name retains text, and clearing both returns the legacy empty recovery format", async () => {
  const f = fixture(); f.journal.update([buffer()],[nameBuffer()]); await f.editor.flushRecovery();
  assert.equal(f.record?.payload.schemaVersion,3);
  f.journal.update([buffer()],[]); await f.editor.flushRecovery();
  assert.equal(f.record?.payload.schemaVersion,1); assert.equal(f.record?.payload.nameBuffers,undefined);
  assert.deepEqual(f.record?.payload.buffers,[buffer()]);
  f.journal.update([],[]); await f.editor.flushRecovery();
  assert.deepEqual(f.record?.payload.buffers,[]); assert.equal(f.record?.payload.nameBuffers,undefined);
  assert.equal(f.calls.some(call => call.kind === "body"),false);
});
