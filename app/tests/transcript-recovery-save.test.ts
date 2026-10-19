import { test } from "node:test";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { TranscriptRecoverySaveQueue, type RecoveryContext, type RecoverySaveRequest, type TranscriptRecovery } from "../src/transcript-recovery-save.ts";
import type { TextEditSession } from "../src/transcript-edit-session.ts";
import type { SpeakerNameSession } from "../src/speaker-name-edit.ts";

type Payload = { text: string };
const hash = (version: number) => version.toString(16).padStart(64,"0");
const context: RecoveryContext<Payload> = { meetingId: "meeting", draftId: "draft", baseAnalysisId: "base",
  lockVersion: 0, contentHash: hash(100), payload: { text: "確定本文" } };
const buffer = (value: string, id = "u"): TextEditSession => ({ utteranceId: id, text: "確定本文", speakerIds: [], hadOverride: false, value });
const name = (value: string, id = "s"): SpeakerNameSession => ({ speakerId: id, name: "田中", hadOverride: true, value });

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes,no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function result(req: RecoverySaveRequest, base = context): TranscriptRecovery<Payload> {
  return { meetingId: req.meetingId, draftId: req.draftId, baseAnalysisId: req.baseAnalysisId,
    operationId: req.operationId, baseDraftLockVersion: req.expectedDraftLockVersion,
    baseDraftContentHash: base.contentHash, contentHash: hash(req.expectedRecoveryLockVersion + 1),
    lockVersion: req.expectedRecoveryLockVersion + 1, updatedAt: 1, matchesDraft: true,
    currentDraftLockVersion: base.lockVersion, currentDraftContentHash: base.contentHash, currentContext: true,
    payload: { ...{ nameBuffers: [], discardedFrom: null }, schemaVersion: 4, basePayload: structuredClone(base.payload), buffers: structuredClone(req.buffers),
      ...(req.nameBuffers?.length ? { nameBuffers: structuredClone(req.nameBuffers) } : {}) } };
}
function fixture(options: { persist?: (req: RecoverySaveRequest) => Promise<TranscriptRecovery<Payload>>;
  read?: () => Promise<TranscriptRecovery<Payload> | null> } = {}) {
  const calls: RecoverySaveRequest[] = [], reads: RecoveryContext<Payload>[] = [];
  let record: TranscriptRecovery<Payload> | null = null;
  const queue = new TranscriptRecoverySaveQueue<Payload>({
    persist: async req => { calls.push(req); record = await (options.persist?.(req) ?? Promise.resolve(result(req))); return record; },
    read: async ctx => { reads.push(ctx); return options.read ? options.read() : record; },
    operationId: randomUUID, normalizePayload: p => ({ text: p.text }),
  });
  queue.load(context,null);
  return { queue, calls, reads, setRecord: (value: TranscriptRecovery<Payload> | null) => { record = value; } };
}

const newerContext: RecoveryContext<Payload> = { ...context,lockVersion: 2,contentHash: hash(102),payload: { text: "別操作の保存本文" } };
test("a reloaded current body can explicitly review older durable input without relaxing unchanged-base pending CAS", async () => {
  const f = fixture(), record = oldRecordOnNewBase(); f.setRecord(record); f.queue.load(newerContext,record);
  const review = await f.queue.inspectRebase(newerContext);
  await f.queue.acceptRebase(review,[buffer("選択")],[]);
  assert.equal(f.calls.length,0); assert.equal(f.queue.snapshot().review,null); assert.equal(f.queue.snapshot().context?.lockVersion,2);
  const fresh = fixture(); await assert.rejects(fresh.queue.inspectRebase(context));
});
test("external local-payload proof is checked again after the final recovery read", async () => {
  let wait: ReturnType<typeof deferred<TranscriptRecovery<Payload> | null>> | null = null, valid = true;
  const f = fixture({ read: async () => wait ? wait.promise : null }), review = await f.queue.inspectRebase(newerContext);
  wait = deferred(); const accepting = f.queue.acceptRebase(review,[],[],() => valid);
  valid = false; wait.resolve(null); await assert.rejects(accepting,/比較後/);
  assert.equal(f.queue.snapshot().context?.lockVersion,0); assert.equal(f.calls.length,0);
});
function oldRecordOnNewBase() {
  const record = result({ meetingId: context.meetingId,draftId: context.draftId,baseAnalysisId: context.baseAnalysisId,
    operationId: randomUUID(),expectedDraftLockVersion: 0,expectedRecoveryLockVersion: 0,buffers: [buffer("別操作の回復本文")],nameBuffers: [name("別操作の回復名")] });
  return { ...record,currentDraftLockVersion: newerContext.lockVersion,currentDraftContentHash: newerContext.contentHash,matchesDraft: false };
}
test("advanced-base inspection is immutable and read-only, exposing local input and the complete stored mixed record", async () => {
  const f = fixture(), record = oldRecordOnNewBase(); f.setRecord(record);
  f.queue.update([buffer("手元の本文")],[name("手元の名")]);
  const before = f.queue.snapshot(), review = await f.queue.inspectRebase(newerContext);
  assert.equal(f.calls.length,0); assert.equal(f.reads.length,1);
  assert.deepEqual(f.queue.snapshot(),before);
  assert.deepEqual(review.record,record); assert.deepEqual(review.buffers,[buffer("手元の本文")]);
  assert.deepEqual(review.nameBuffers,[name("手元の名")]);
  assert.throws(() => { review.context.payload.text = "改変"; },TypeError);
  assert.throws(() => { review.record!.payload.nameBuffers![0].value = "改変"; },TypeError);
});
test("explicit reviewed rebase rereads its exact record and retires an obsolete request without sending it or new input", async () => {
  const f = fixture({ persist: async req => { if(req.expectedDraftLockVersion === 0) throw Error("旧回復版競合"); return result(req,newerContext); } });
  f.queue.update([buffer("元本文")],[name("元名")]); await assert.rejects(f.queue.flush());
  const old = f.queue.snapshot().pendingRequest; f.setRecord(oldRecordOnNewBase());
  f.queue.update([buffer("最新手元本文")],[name("最新手元名")]);
  const review = await f.queue.inspectRebase(newerContext);
  assert.equal(f.queue.snapshot().pendingRequest,old);
  const selectedText = { ...buffer("選んだ本文"),text: newerContext.payload.text,hadOverride: true };
  await f.queue.acceptRebase(review,[selectedText],[name("選んだ名")]);
  assert.equal(f.calls.length,1); assert.equal(f.reads.length,2); assert.equal(f.queue.snapshot().pendingRequest,null);
  assert.equal(f.queue.snapshot().context?.lockVersion,2); assert.deepEqual(f.queue.snapshot().buffers,[selectedText]);
  assert.equal(f.queue.snapshot().dirty,true); assert.equal(f.queue.snapshot().review,null);
  await f.queue.flush();
  assert.equal(f.calls.length,2); assert.equal(f.calls[1].expectedDraftLockVersion,2);
  assert.equal(f.calls[1].expectedRecoveryLockVersion,review.record!.lockVersion);
  assert.notEqual(f.calls[1].operationId,old!.operationId);
});
test("rebase requires a strictly advanced valid same-owner context and refuses stale or foreign record metadata", async () => {
  for(const patch of [{ lockVersion: 0 },{ lockVersion: -1 },{ lockVersion: Number.MAX_SAFE_INTEGER + 1 },{ contentHash: "invalid" },{ meetingId: "other" },{ draftId: "other" },{ baseAnalysisId: "other" }]) {
    const f = fixture(); f.queue.update([buffer("保持")],[name("保持")]);
    await assert.rejects(f.queue.inspectRebase({ ...newerContext,...patch }));
    assert.equal(f.reads.length,0); assert.equal(f.calls.length,0); assert.equal(f.queue.snapshot().buffers[0].value,"保持");
  }
  for(const patch of [{ currentContext: false },{ currentDraftLockVersion: 1 },{ currentDraftContentHash: hash(99) },{ draftId: "other" },{ baseDraftLockVersion: 3 }]) {
    const f = fixture(); f.setRecord({ ...oldRecordOnNewBase(),...patch });
    await assert.rejects(f.queue.inspectRebase(newerContext)); assert.equal(f.calls.length,0);
    assert.equal(f.queue.snapshot().context?.lockVersion,0);
  }
});
test("changed mixed input, pending request and forged or superseded review tokens cannot release the original context", async () => {
  const f = fixture(); f.setRecord(oldRecordOnNewBase()); f.queue.update([buffer("旧入力")],[name("旧名")]);
  const review = await f.queue.inspectRebase(newerContext);
  await assert.rejects(f.queue.acceptRebase(structuredClone(review),[],[]));
  f.queue.update([buffer("新入力")],[name("新名")]);
  await assert.rejects(f.queue.acceptRebase(review,[],[]));
  const newer = await f.queue.inspectRebase(newerContext), last = await f.queue.inspectRebase(newerContext);
  await assert.rejects(f.queue.acceptRebase(newer,[],[]));
  await f.queue.acceptRebase(last,[buffer("新入力")],[name("新名")]);
  assert.equal(f.calls.length,0); assert.equal(f.queue.snapshot().buffers[0].value,"新入力");
});
test("a changed or disappeared stored record after comparison keeps input and the immutable pending request", async () => {
  for(const change of ["name","text","nonce","version","hash","missing"]) {
    const f = fixture({ persist: async () => { throw Error("競合"); } });
    f.queue.update([buffer("手元")],[name("手元")]); await assert.rejects(f.queue.flush());
    const pending = f.queue.snapshot().pendingRequest, record = oldRecordOnNewBase(); f.setRecord(record);
    const review = await f.queue.inspectRebase(newerContext);
    const changed = structuredClone(record);
    if(change === "name") changed.payload.nameBuffers![0].value = "別の名";
    if(change === "text") changed.payload.buffers[0].value = "別の本文";
    if(change === "nonce") changed.operationId = randomUUID();
    if(change === "version") changed.lockVersion++;
    if(change === "hash") changed.contentHash = hash(99);
    f.setRecord(change === "missing" ? null : changed);
    await assert.rejects(f.queue.acceptRebase(review,[],[]),/比較後/);
    assert.equal(f.queue.snapshot().pendingRequest,pending); assert.equal(f.queue.snapshot().context?.lockVersion,0);
    assert.equal(f.queue.snapshot().buffers[0].value,"手元"); assert.equal(f.calls.length,1);
  }
});
test("rebase inspection and acceptance detect input changing during their reads and retain both types", async () => {
  for(const phase of ["inspect","accept"]) {
    let delayed = false; const wait = deferred<TranscriptRecovery<Payload> | null>();
    const f = fixture({ read: async () => delayed ? wait.promise : oldRecordOnNewBase() });
    f.queue.update([buffer("元")],[name("元")]);
    const review = phase === "accept" ? await f.queue.inspectRebase(newerContext) : null;
    delayed = true;
    const running = review ? f.queue.acceptRebase(review,[],[]) : f.queue.inspectRebase(newerContext);
    await Promise.resolve(); f.queue.update([buffer("途中の本文")],[name("途中の名")]); wait.resolve(oldRecordOnNewBase());
    await assert.rejects(running); assert.equal(f.queue.snapshot().context?.lockVersion,0);
    assert.equal(f.queue.snapshot().nameBuffers[0].value,"途中の名"); assert.equal(f.calls.length,0);
  }
});
test("rebase validates selected mixed input and UTF-8 capacity before replacing state or doing its final read", async () => {
  for(const [texts,names] of [
    [[buffer("x".repeat(20_001))],[]], [[],[name("x".repeat(201))]],
    [[buffer("a"),buffer("b")],[]], [[],[name("a"),name("b")]],
    [Array.from({ length: 128 },(_,i) => buffer("界".repeat(20_000),String(i))),[]],
  ] as [TextEditSession[],SpeakerNameSession[]][]) {
    const f = fixture(); f.setRecord(oldRecordOnNewBase()); f.queue.update([buffer("保持")],[name("保持")]);
    const review = await f.queue.inspectRebase(newerContext);
    await assert.rejects(f.queue.acceptRebase(review,texts,names));
    assert.equal(f.reads.length,1); assert.equal(f.calls.length,0); assert.equal(f.queue.snapshot().context?.lockVersion,0);
    assert.equal(f.queue.snapshot().buffers[0].value,"保持");
  }
});
test("rebase refuses an active save and disposal during final read without accepting late state", async () => {
  const wait = deferred<TranscriptRecovery<Payload>>();
  const f = fixture({ persist: async () => wait.promise }); f.queue.update([buffer("保持")]);
  const saving = f.queue.flush(); await Promise.resolve();
  await assert.rejects(f.queue.inspectRebase(newerContext)); wait.resolve(result(f.calls[0])); await saving;
  let delay = false; const readWait = deferred<TranscriptRecovery<Payload> | null>();
  const g = fixture({ read: async () => delay ? readWait.promise : oldRecordOnNewBase() });
  const review = await g.queue.inspectRebase(newerContext); delay = true;
  const accepting = g.queue.acceptRebase(review,[],[]); g.queue.dispose(); readWait.resolve(oldRecordOnNewBase());
  await assert.rejects(accepting,/終了/); assert.equal(g.queue.snapshot().context?.lockVersion,0);
});

test("name-only recovery is normalized immutable and uses the current format for both names and text", async () => {
  const { queue,calls } = fixture();
  const input = name("入力中","b");
  queue.update([],[input,{ ...name("別名","a"),name: null,hadOverride: false }]);
  input.value = "外部変更";
  await queue.flush();
  assert.deepEqual(calls[0].buffers,[]);
  assert.deepEqual(calls[0].nameBuffers?.map(n => n.speakerId),["a","b"]);
  assert.equal(calls[0].nameBuffers?.[1].value,"入力中");
  assert.throws(() => { calls[0].nameBuffers![0].value = "変更"; },TypeError);
  assert.equal(queue.snapshot().saved?.payload.schemaVersion,4);
  assert.equal(queue.snapshot().dirty,false);
  queue.update([buffer("本文だけ")]); await queue.flush();
  assert.equal(Object.hasOwn(calls[1],"nameBuffers"),true);
  assert.equal(Object.hasOwn(queue.snapshot().saved!.payload,"nameBuffers"),true);
  assert.equal(queue.snapshot().saved?.payload.schemaVersion,4);
});

test("name and text changes coalesce under one CAS without replacing an in-flight request", async () => {
  const wait = deferred<TranscriptRecovery<Payload>>();
  const { queue,calls } = fixture({ persist: req => calls.length === 1 ? wait.promise : Promise.resolve(result(req)) });
  queue.update([buffer("旧本文")],[name("旧名")]);
  const saving = queue.flush(); await Promise.resolve();
  queue.update([buffer("新本文")],[name("新名")]);
  assert.equal(calls[0].nameBuffers?.[0].value,"旧名");
  wait.resolve(result(calls[0])); await saving;
  assert.equal(calls.length,2);
  assert.equal(calls[1].expectedRecoveryLockVersion,1);
  assert.equal(calls[1].buffers[0].value,"新本文");
  assert.equal(calls[1].nameBuffers?.[0].value,"新名");
  assert.equal(queue.snapshot().dirty,false);
});

test("clearing names during a delayed write saves an empty next revision and never revives them", async () => {
  const wait = deferred<TranscriptRecovery<Payload>>();
  const { queue,calls } = fixture({ persist: req => calls.length === 1 ? wait.promise : Promise.resolve(result(req)) });
  queue.update([],[name("取消前")]);
  const saving = queue.flush(); await Promise.resolve(); queue.update([]);
  wait.resolve(result(calls[0])); await saving;
  assert.equal(calls.length,2); assert.equal(calls[1].expectedRecoveryLockVersion,1);
  assert.equal(Object.hasOwn(calls[1],"nameBuffers"),true);
  assert.equal(queue.snapshot().saved?.payload.schemaVersion,4);
  assert.deepEqual(queue.snapshot().nameBuffers,[]);
  assert.equal(queue.snapshot().dirty,false);
});

test("lost-ack read-back acknowledges only the old mixed request and preserves newer names without sending them", async () => {
  const { queue,calls,reads,setRecord } = fixture({ persist: async req => { setRecord(result(req)); throw Error("応答喪失"); } });
  queue.update([buffer("保存済み本文")],[name("保存済み名")]);
  await assert.rejects(queue.flush());
  const pending = queue.snapshot().pendingRequest;
  queue.update([buffer("未送信本文")],[name("未送信名")]);
  assert.equal(pending?.nameBuffers?.[0].value,"保存済み名");
  assert.equal(await queue.recheck(),"saved");
  assert.equal(calls.length,1); assert.equal(reads.length,1);
  assert.equal(queue.snapshot().nameBuffers[0].value,"未送信名");
  assert.equal(queue.snapshot().buffers[0].value,"未送信本文");
  assert.equal(queue.snapshot().dirty,true);
});

test("name retry preserves the original baseline override and value before saving newer input as a new CAS", async () => {
  let fail = true;
  const { queue,calls } = fixture({ persist: async req => { if(fail) throw Error("未保存"); return result(req); } });
  queue.update([],[name("旧名")]); await assert.rejects(queue.flush());
  queue.update([],[{ ...name("新名"),name: null,hadOverride: false }]);
  fail = false; await queue.retry();
  assert.equal(calls.length,3); assert.equal(calls[1],calls[0]);
  assert.deepEqual(calls[1].nameBuffers,[name("旧名")]);
  assert.deepEqual(calls[2].nameBuffers,[{ ...name("新名"),name: null,hadOverride: false }]);
  assert.equal(calls[2].expectedRecoveryLockVersion,1);
});

test("name-only reload requires explicit fresh-read resume and stale names are never adopted", async () => {
  for(const stale of [false,true]) {
    const record = result({ ...context,operationId: randomUUID(),expectedDraftLockVersion: 0,expectedRecoveryLockVersion: 0,
      buffers: [],nameBuffers: [name("回復名")] });
    if(stale) { record.matchesDraft = false; record.currentDraftLockVersion = 1; }
    const { queue,calls,reads,setRecord } = fixture();
    queue.load(context,record); setRecord(record);
    assert.deepEqual(queue.snapshot().nameBuffers,[]); assert.equal(queue.snapshot().dirty,false);
    assert.ok(queue.snapshot().review); await assert.rejects(queue.flush(),/確認/);
    if(stale) { await assert.rejects(queue.resume(),/自動適用せず/); assert.ok(queue.snapshot().review); }
    else { await queue.resume(); assert.deepEqual(queue.snapshot().nameBuffers,[name("回復名")]); assert.equal(queue.snapshot().dirty,false); }
    assert.equal(calls.length,0); assert.equal(reads.length,1);
  }
});

test("name result mismatches never clear the immutable request or the latest input", async () => {
  for(const field of ["missing","value","baseline","override","owner","extra","schema"]) {
    const { queue,calls,setRecord } = fixture({ persist: async req => {
      const bad = result(req);
      switch(field) {
        case "missing": delete bad.payload.nameBuffers; break;
        case "value": bad.payload.nameBuffers![0].value = "別名"; break;
        case "baseline": bad.payload.nameBuffers![0].name = null; break;
        case "override": bad.payload.nameBuffers![0].hadOverride = false; break;
        case "owner": bad.payload.nameBuffers![0].speakerId = "別話者"; break;
        case "extra": bad.payload.nameBuffers!.push(name("追加","z")); break;
        default: bad.payload.schemaVersion = 1;
      }
      setRecord(bad); return bad;
    } });
    queue.update([],[name("保持名")]); await assert.rejects(queue.flush(),/一致/);
    await assert.rejects(queue.recheck(),/別の回復保存/);
    assert.ok(queue.snapshot().pendingRequest,field); assert.equal(calls.length,1,field);
    assert.equal(queue.snapshot().saved,null,field); assert.equal(queue.snapshot().nameBuffers[0].value,"保持名",field);
  }
});

test("invalid name limits are correctable before a nonce while 100 names of 200 Unicode characters are supported", async () => {
  for(const values of [Array.from({ length: 101 },(_,i) => name("名",`s-${i}`)),[name("😀".repeat(201))],
    [{ ...name("名"),name: "😀".repeat(201) }],[name("名"),name("重複")]]) {
    const { queue,calls } = fixture(); queue.update([],values); await assert.rejects(queue.flush(),/話者名/);
    assert.equal(calls.length,0); assert.equal(queue.snapshot().pendingRequest,null);
    assert.equal(queue.snapshot().nameBuffers.length,values.length);
    queue.update([],[{ ...name(""),name: null,hadOverride: false }]); await queue.retry();
    assert.equal(calls.length,1);
  }
  const { queue } = fixture();
  queue.update([],Array.from({ length: 100 },(_,i) => ({ ...name("😀".repeat(200),`s-${i}`),name: "😀".repeat(200) })));
  await queue.flush(); assert.equal(queue.snapshot().saved?.payload.nameBuffers?.length,100);
});

test("published draft advancement rebinds names preserving their editing-start baseline", async () => {
  let base = context;
  const { queue,calls } = fixture({ persist: async req => result(req,base) });
  queue.update([],[name("未確定名")]); await queue.flush();
  base = { ...context,lockVersion: 1,contentHash: hash(101),payload: { text: "確定本文の更新" } };
  queue.advance(base); assert.equal(queue.snapshot().dirty,true); await queue.flush();
  assert.equal(calls[1].expectedDraftLockVersion,1); assert.equal(calls[1].expectedRecoveryLockVersion,1);
  assert.deepEqual(calls[1].nameBuffers,[name("未確定名")]);
});

test("name JSON bytes count toward the shared UTF-8 capacity before assigning another nonce", async () => {
  const { queue,calls } = fixture();
  const text = Array.from({ length: 26 },(_,i) => ({ ...buffer("😀".repeat(20_000),`u-${i}`),text: "😀".repeat(20_000) }));
  queue.update(text); await queue.flush();
  const values = Array.from({ length: 100 },(_,i) => ({ ...name("😀".repeat(200),`s-${i}`),name: "😀".repeat(200) }));
  queue.update(text,values); await assert.rejects(queue.flush(),/4MiB/);
  assert.equal(calls.length,1); assert.equal(queue.snapshot().pendingRequest,null);
  assert.equal(queue.snapshot().buffers.length,26); assert.equal(queue.snapshot().nameBuffers.length,100);
  queue.update(text,[name("短い名")]); await queue.retry();
  assert.equal(calls.length,2); assert.equal(calls[1].expectedRecoveryLockVersion,1);
});

test("name record format and sorted identity checks reject malformed reviews before replacing local context", () => {
  for(const field of ["versionOne","proof","duplicates","unsorted","unknown"]) {
    const record = result({ ...context,operationId: randomUUID(),expectedDraftLockVersion: 0,expectedRecoveryLockVersion: 0,
      buffers: [],nameBuffers: [name("名","a"),name("別名","b")] });
    switch(field) {
      case "empty": record.payload.nameBuffers = []; break;
      case "versionOne": record.payload.schemaVersion = 1; break;
      case "proof": record.payload.schemaVersion = 2; record.payload.discardedFrom = { contentHash: hash(1),lockVersion: 1 }; record.lockVersion = 2; break;
      case "duplicates": record.payload.nameBuffers![1].speakerId = "a"; break;
      case "unsorted": record.payload.nameBuffers!.reverse(); break;
      default: Object.assign(record.payload.nameBuffers![0],{ unknown: true });
    }
    const { queue } = fixture();
    assert.throws(() => queue.load(context,record),/所有または形式/,field);
    assert.equal(queue.snapshot().saved,null); assert.equal(queue.snapshot().review,null);
  }
});

test("invalid local buffer limits retain input without freezing an unretryable request", async () => {
  for(const values of [Array.from({ length: 129 },(_,i) => buffer("入力",`u-${i}`)),[buffer("a".repeat(20_001))],
    [buffer("入力"),buffer("別入力")],[{ ...buffer("入力"),speakerIds: ["s","s"] }]]) {
    const { queue,calls } = fixture(); queue.update(values);
    await assert.rejects(queue.flush(),/上限または重複/);
    assert.equal(queue.snapshot().pendingRequest,null); assert.equal(calls.length,0);
    assert.equal(queue.snapshot().buffers.length,values.length);
    queue.update([buffer("修正後")]); await queue.retry(); assert.equal(calls.length,1);
  }
});
test("UTF-8 recovery capacity is checked before creating a nonce and is correctable", async () => {
  const { queue,calls } = fixture();
  queue.update(Array.from({ length: 100 },(_,i) => ({ ...buffer("あ".repeat(20_000),`u-${i}`),text: "あ".repeat(20_000) })));
  await assert.rejects(queue.flush(),/4MiB/); assert.equal(queue.snapshot().pendingRequest,null); assert.equal(calls.length,0);
  queue.update([buffer("小さい入力")]); await queue.retry(); assert.equal(calls.length,1);
});

test("recovery coalesces typing after an immutable in-flight request and increments CAS", async () => {
  const wait = deferred<TranscriptRecovery<Payload>>();
  const { queue, calls } = fixture({ persist: req => calls.length === 1 ? wait.promise : Promise.resolve(result(req)) });
  const input = buffer("先行入力");
  queue.update([input]);
  const saving = queue.flush();
  await Promise.resolve();
  input.value = "外部変更";
  queue.update([buffer("新入力")]);
  assert.equal(calls[0].buffers[0].value,"先行入力");
  assert.throws(() => { calls[0].buffers[0].value = "不正な変更"; },TypeError);
  wait.resolve(result(calls[0]));
  await saving;
  assert.equal(calls.length,2);
  assert.equal(calls[1].expectedRecoveryLockVersion,1);
  assert.equal(calls[1].buffers[0].value,"新入力");
  assert.notEqual(calls[0].operationId,calls[1].operationId);
  assert.equal(queue.snapshot().dirty,false);
});

test("clearing input during a recovery save waits for the older write then saves an empty next version", async () => {
  const wait = deferred<TranscriptRecovery<Payload>>();
  const { queue,calls } = fixture({ persist: req => calls.length === 1 ? wait.promise : Promise.resolve(result(req)) });
  queue.update([buffer("取消前の入力")]);
  const saving = queue.flush();
  await Promise.resolve();
  queue.update([]);
  assert.equal(queue.snapshot().buffers.length,0);
  wait.resolve(result(calls[0]));
  await saving;
  assert.equal(calls.length,2);
  assert.equal(calls[1].expectedRecoveryLockVersion,1);
  assert.deepEqual(calls[1].buffers,[]);
  assert.equal(queue.snapshot().saved?.lockVersion,2);
  assert.equal(queue.snapshot().buffers.length,0);
  assert.equal(queue.snapshot().dirty,false);
});

test("ambiguous recovery failure keeps latest typing and blocks automatic retry and load", async () => {
  const { queue, calls } = fixture({ persist: async () => { throw Error("応答不明"); } });
  queue.update([buffer("保持する入力")]);
  await assert.rejects(queue.flush(),/応答不明/);
  const pending = queue.snapshot().pendingRequest;
  queue.update([buffer("追加の入力")]);
  await assert.rejects(queue.flush(),/応答不明/);
  assert.equal(calls.length,1);
  assert.equal(queue.snapshot().pendingRequest,pending);
  assert.equal(queue.snapshot().buffers[0].value,"追加の入力");
  assert.throws(() => queue.load(context,null),/結果未確認/);
  assert.throws(() => queue.advance({ ...context, lockVersion: 1 }),/未確認/);
});

test("read-only lost-ack verification acknowledges only the exact request without writing newer input", async () => {
  const { queue, calls, reads, setRecord } = fixture({ persist: async req => { setRecord(result(req)); throw Error("応答喪失"); } });
  queue.update([buffer("保存された入力")]);
  await assert.rejects(queue.flush(),/応答喪失/);
  queue.update([buffer("未送信の入力")]);
  assert.equal(await queue.recheck(),"saved");
  assert.equal(reads.length,1);
  assert.equal(calls.length,1);
  assert.equal(queue.snapshot().pendingRequest,null);
  assert.equal(queue.snapshot().error,null);
  assert.equal(queue.snapshot().dirty,true);
  assert.equal(queue.snapshot().buffers[0].value,"未送信の入力");
});

test("unchanged head is not acknowledgement and explicit retry uses the original operation and input", async () => {
  let fail = true;
  const { queue, calls, reads } = fixture({ persist: async req => { if(fail) throw Error("未保存"); return result(req); },read: async () => null });
  queue.update([buffer("旧要求")]);
  await assert.rejects(queue.flush());
  queue.update([buffer("最新入力")]);
  assert.equal(await queue.recheck(),"unchanged");
  assert.equal(calls.length,1);
  assert.equal(reads.length,1);
  assert.ok(queue.snapshot().pendingRequest);
  await assert.rejects(queue.flush());
  fail = false;
  await queue.retry();
  assert.equal(calls.length,3);
  assert.equal(calls[1],calls[0]);
  assert.equal(calls[1].buffers[0].value,"旧要求");
  assert.equal(calls[2].buffers[0].value,"最新入力");
  assert.equal(calls[2].expectedRecoveryLockVersion,1);
});

test("mismatched recovery acknowledgements never advance the saved head or lose input", async () => {
  for (const field of ["owner","operation","version","baseHash","draftVersion","current","matches","value","schema","hash","payload","unsafe"]) {
    const { queue, calls, setRecord } = fixture({ persist: async req => {
      const bad = result(req);
      switch(field) {
        case "owner": bad.meetingId = "foreign"; break;
        case "operation": bad.operationId = randomUUID(); break;
        case "version": bad.lockVersion++; break;
        case "baseHash": bad.baseDraftContentHash = hash(200); break;
        case "draftVersion": bad.currentDraftLockVersion++; break;
        case "current": bad.currentContext = false; break;
        case "matches": bad.matchesDraft = false; break;
        case "value": bad.payload.buffers[0].value = "別の入力"; break;
        case "schema": bad.payload.schemaVersion = 3; break;
        case "hash": bad.contentHash = "invalid"; break;
        case "payload": bad.payload.basePayload.text = "別の基底"; break;
        default: bad.lockVersion = Number.MAX_SAFE_INTEGER + 1;
      }
      setRecord(bad);
      return bad;
    } });
    queue.update([buffer("保持")]);
    await assert.rejects(queue.flush(),/一致/);
    await assert.rejects(queue.recheck(),/更新/);
    assert.equal(calls.length,1,field);
    assert.equal(queue.snapshot().saved,null,field);
    assert.equal(queue.snapshot().buffers[0].value,"保持",field);
    assert.ok(queue.snapshot().pendingRequest,field);
  }
});

test("a newer foreign operation cannot be mistaken for an unchanged read-only result", async () => {
  const { queue, calls, setRecord } = fixture({ persist: async () => { throw Error("喪失"); } });
  queue.update([buffer("保持")]);
  await assert.rejects(queue.flush());
  const newHead = result({ ...calls[0],operationId: randomUUID(), expectedRecoveryLockVersion: 1 });
  setRecord(newHead);
  await assert.rejects(queue.recheck(),/別の回復保存/);
  assert.ok(queue.snapshot().pendingRequest);
  assert.equal(calls.length,1);
});

test("stored input requires explicit fresh-read resume and does not publish or auto-save", async () => {
  const record = result({ meetingId: context.meetingId,draftId: context.draftId,baseAnalysisId: context.baseAnalysisId,
    operationId: randomUUID(),expectedDraftLockVersion: 0,expectedRecoveryLockVersion: 0,buffers: [buffer("回復入力")] });
  const { queue, calls, reads, setRecord } = fixture();
  setRecord(record);
  queue.load(context,record);
  assert.equal(queue.snapshot().buffers.length,0);
  assert.ok(queue.snapshot().review);
  assert.equal(queue.snapshot().dirty,false);
  assert.throws(() => queue.update([buffer("上書き")]),/確認/);
  await assert.rejects(queue.flush(),/確認/);
  await queue.resume();
  assert.equal(reads.length,1);
  assert.equal(calls.length,0);
  assert.equal(queue.snapshot().buffers[0].value,"回復入力");
  assert.equal(queue.snapshot().review,null);
  assert.equal(queue.snapshot().dirty,false);
  await queue.flush();
  assert.equal(calls.length,0);
});

test("stale archived or changed review cannot be silently resumed", async () => {
  for (const field of ["inactive","stale","newHead","changedPayload","missing"]) {
    const record = result({ ...context, operationId: randomUUID(),expectedDraftLockVersion: 0,
      expectedRecoveryLockVersion: 0,buffers: [buffer("旧入力")] });
    const fresh = structuredClone(record);
    switch(field) {
      case "inactive": fresh.currentContext = false; break;
      case "stale": fresh.matchesDraft = false; fresh.currentDraftLockVersion++; break;
      case "newHead": fresh.contentHash = hash(2); fresh.lockVersion++; break;
      case "changedPayload": fresh.payload.buffers[0].value = "確認後の入力"; break;
    }
    const { queue,calls,setRecord } = fixture();
    queue.load(context,record);
    setRecord(field === "missing" ? null : fresh);
    await assert.rejects(queue.resume(),/自動適用せず/);
    assert.equal(queue.snapshot().buffers.length,0,field);
    assert.ok(queue.snapshot().review,field);
    assert.equal(calls.length,0,field);
  }
});

test("advancing published draft rebinds the remaining buffers only after the previous recovery save", async () => {
  let base = context;
  const { queue,calls } = fixture({ persist: async req => result(req,base) });
  queue.update([buffer("入力")]);
  await queue.flush();
  base = { ...context,lockVersion: 1,contentHash: hash(101),payload: { text: "新しい確定本文" } };
  queue.advance(base);
  assert.equal(queue.snapshot().dirty,true);
  await queue.flush();
  assert.equal(calls[1].expectedDraftLockVersion,1);
  assert.equal(calls[1].expectedRecoveryLockVersion,1);
  assert.equal(queue.snapshot().saved?.baseDraftContentHash,base.contentHash);
  assert.equal(queue.snapshot().saved?.payload.buffers[0].text,"確定本文");
  queue.update([]);
  await queue.flush();
  assert.equal(calls[2].expectedRecoveryLockVersion,2);
  assert.deepEqual(calls[2].buffers,[]);
  assert.equal(queue.snapshot().dirty,false);
});

test("dispose rejects a late acknowledgement and sends no coalesced follow-up writes", async () => {
  const wait = deferred<TranscriptRecovery<Payload>>();
  const { queue,calls } = fixture({ persist: () => wait.promise });
  queue.update([buffer("先行")]);
  const saving = queue.flush();
  await Promise.resolve();
  queue.update([buffer("後続")]);
  queue.dispose();
  wait.resolve(result(calls[0]));
  await assert.rejects(saving,/終了/);
  assert.equal(calls.length,1);
  assert.equal(queue.snapshot().pendingRequest?.buffers[0].value,"先行");
  assert.equal(queue.snapshot().buffers[0].value,"後続");
  await assert.rejects(queue.retry(),/終了/);
  assert.throws(() => queue.load(context,null),/終了/);
});

test("reviewed empty tombstone keeps its CAS version and cannot count as a normal save acknowledgement", async () => {
  const record = result({ ...context,operationId: randomUUID(),expectedDraftLockVersion: 0,
    expectedRecoveryLockVersion: 1,buffers: [] });
  record.payload.schemaVersion = 4;
  record.payload.discardedFrom = { contentHash: hash(1),lockVersion: 1 };
  const { queue,calls } = fixture();
  queue.load(context,record);
  await queue.flush();
  assert.equal(calls.length,0);
  queue.update([buffer("破棄後の新入力")]);
  await queue.flush();
  assert.equal(calls[0].expectedRecoveryLockVersion,2);
  assert.equal(queue.snapshot().saved?.payload.schemaVersion,4);
});

test("normal save cannot accept a disguised reviewed-discard acknowledgement", async () => {
  const { queue,setRecord } = fixture({ persist: async req => {
    const record = result(req); record.payload.schemaVersion = 2;
    record.payload.buffers = []; record.payload.discardedFrom = { contentHash: hash(1),lockVersion: req.expectedRecoveryLockVersion };
    setRecord(record); return record;
  } });
  queue.update([buffer("保持")]);
  await assert.rejects(queue.flush(),/一致/);
  await assert.rejects(queue.recheck(),/更新/);
  assert.equal(queue.snapshot().buffers[0].value,"保持");
});

test("unsafe versions invalid owner and unknown formats are refused before load or save", async () => {
  const { queue,calls } = fixture();
  assert.throws(() => queue.load({ ...context,lockVersion: Number.MAX_SAFE_INTEGER + 1 },null),/条件/);
  const record = result({ ...context,operationId: randomUUID(),expectedDraftLockVersion: 0,
    expectedRecoveryLockVersion: 0,buffers: [] });
  for(const patch of [{ meetingId: "foreign" },{ lockVersion: 0 },{ contentHash: "INVALID" },{ operationId: "invalid" },
    { baseDraftLockVersion: 1 },{ payload: { ...record.payload,schemaVersion: 3 } }]) {
    assert.throws(() => queue.load(context,{ ...record,...patch }),/所有または形式/);
  }
  record.lockVersion = Number.MAX_SAFE_INTEGER;
  queue.load(context,record);
  queue.update([buffer("保持")]);
  await assert.rejects(queue.flush(),/安全な範囲/);
  assert.equal(calls.length,0);
  assert.equal(queue.snapshot().buffers[0].value,"保持");
});

test("invalid operation identity is refused before any persistence call and keeps typed input", async () => {
  let writes = 0;
  const queue = new TranscriptRecoverySaveQueue<Payload>({
    persist: async req => { writes++; return result(req); },read: async () => null,
    operationId: () => "invalid",normalizePayload: p => p,
  });
  queue.load(context,null);
  queue.update([buffer("保持する入力")]);
  await assert.rejects(queue.flush(),/操作ID/);
  assert.equal(writes,0);
  assert.equal(queue.snapshot().buffers[0].value,"保持する入力");
});

test("in-flight marker precedes the persistence callback and blocks context replacement", async () => {
  const { queue } = fixture({ persist: async req => {
    assert.equal(queue.snapshot().saving,true);
    assert.throws(() => queue.load(context,null),/保存中/);
    assert.throws(() => queue.advance({ ...context,lockVersion: 1 }),/未確認/);
    return result(req);
  } });
  queue.update([buffer("入力")]);
  await queue.flush();
  assert.equal(queue.snapshot().saving,false);
});

test("read-only recheck racing explicit retry cannot overwrite the later acknowledged state", async () => {
  const wait = deferred<TranscriptRecovery<Payload> | null>();
  let fail = true;
  const { queue,calls } = fixture({ persist: async req => { if(fail) throw Error("不明"); return result(req); },read: () => wait.promise });
  queue.update([buffer("入力")]);
  await assert.rejects(queue.flush());
  const checking = queue.recheck();
  fail = false;
  await queue.retry();
  wait.resolve(null);
  await assert.rejects(checking,/結果確認中/);
  assert.equal(calls[0],calls[1]);
  assert.equal(queue.snapshot().saved?.lockVersion,1);
  assert.equal(queue.snapshot().pendingRequest,null);
});
