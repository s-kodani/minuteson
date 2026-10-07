import { test } from "node:test";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { TranscriptRecoverySaveQueue, type RecoveryContext, type RecoverySaveRequest, type TranscriptRecovery } from "../src/transcript-recovery-save.ts";
import type { TextEditSession } from "../src/transcript-edit-session.ts";

type Payload = { text: string };
const hash = (version: number) => version.toString(16).padStart(64,"0");
const context: RecoveryContext<Payload> = { meetingId: "meeting", draftId: "draft", baseAnalysisId: "base",
  lockVersion: 0, contentHash: hash(100), payload: { text: "確定本文" } };
const buffer = (value: string, id = "u"): TextEditSession => ({ utteranceId: id, text: "確定本文", speakerIds: [], hadOverride: false, value });

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
    payload: { schemaVersion: 1, basePayload: structuredClone(base.payload), buffers: structuredClone(req.buffers) } };
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
  record.payload.schemaVersion = 2;
  record.payload.discardedFrom = { contentHash: hash(1),lockVersion: 1 };
  const { queue,calls } = fixture();
  queue.load(context,record);
  await queue.flush();
  assert.equal(calls.length,0);
  queue.update([buffer("破棄後の新入力")]);
  await queue.flush();
  assert.equal(calls[0].expectedRecoveryLockVersion,2);
  assert.equal(queue.snapshot().saved?.payload.schemaVersion,1);
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
