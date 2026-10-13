import { test } from "node:test";
import assert from "node:assert/strict";
import { SavedEditsCancelled } from "../src/confirmed-action.ts";
import { ConfirmationController } from "../src/confirmation-state.ts";
const options = (action: () => Promise<void>) => ({ title: "削除を確認", target: "固定した対象", message: "削除した入力は復元できません。", confirmLabel: "削除する", action });
const controller = () => new ConfirmationController<object, object>();
test("explicit edit cancellation settles as cancelled without reporting completed or unknown work",async()=>{
  const c=controller();let errors=0;
  const result=c.request({},null,null,{...options(async()=>{throw new SavedEditsCancelled();}),onError:()=>{errors++;}});
  await c.execute(c.getSnapshot()!.id);
  assert.equal(await result,"cancelled");assert.equal(c.getSnapshot(),null);assert.equal(errors,0);
});
test("explicit alternative shares validation and prevents primary execution, double acceptance and dismissal while running",async()=>{
  const c=controller();let primary=0,alternative=0,release!:()=>void;
  const result=c.request({},null,null,{...options(async()=>{primary++;}),alternativeAction:{label:"破棄",action:async()=>{alternative++;await new Promise<void>(resolve=>{release=resolve;});}}});
  const id=c.getSnapshot()!.id;assert.equal(c.getSnapshot()!.alternativeLabel,"破棄");
  const execution=c.execute(id,true);await c.execute(id);c.cancel(id);assert.equal(c.getSnapshot()!.phase,"running");
  release();await execution;assert.equal(await result,"completed");assert.equal(primary,0);assert.equal(alternative,1);
  const absent=c.request({},null,null,options(async()=>{primary++;}));await c.execute(c.getSnapshot()!.id,true);assert.equal(c.getSnapshot()!.phase,"choosing");c.cancel(c.getSnapshot()!.id);await absent;
  const stale=c.request({},null,null,{...options(async()=>{primary++;}),validate:()=>false,alternativeAction:{label:"破棄",action:async()=>{alternative++;}}});
  await c.execute(c.getSnapshot()!.id,true);assert.equal(c.getSnapshot()!.phase,"failed");assert.equal(alternative,1);c.cancel(c.getSnapshot()!.id);assert.equal(await stale,"unconfirmed");
});

test("confirmation never runs before acceptance, cancellation settles without changes", async () => {
  const c = controller(); let calls = 0;
  const result = c.request({}, null, {}, options(async () => { calls++; }));
  const id = c.getSnapshot()!.id; assert.equal(calls, 0);
  c.cancel(id); assert.equal(await result, "cancelled"); assert.equal(calls, 0); assert.equal(c.getSnapshot(), null);
});
test("synchronous running state prevents double execution and dismissal until acknowledgement", async () => {
  const c = controller(); let calls = 0, done!: () => void;
  const result = c.request({}, null, {}, options(async () => { calls++; await new Promise<void>(resolve => { done = resolve; }); }));
  const id = c.getSnapshot()!.id, first = c.execute(id);
  assert.equal(c.getSnapshot()!.phase, "running"); await c.execute(id); c.cancel(id);
  assert.equal(calls, 1); assert.equal(c.getSnapshot()!.phase, "running");
  done(); await first; assert.equal(await result, "completed"); assert.equal(c.getSnapshot(), null);
});
test("a second request is not queued, stacked or implicitly accepted", async () => {
  const c = controller(); let calls = 0;
  const first = c.request({}, null, null, options(async () => { calls++; })); const id = c.getSnapshot()!.id;
  assert.equal(await c.request({}, {}, {}, options(async () => { calls++; })), "cancelled");
  assert.equal(c.getSnapshot()!.id, id); assert.equal(calls, 0); c.cancel(id); await first;
});
test("wrong and stale IDs cannot accept or cancel another request", async () => {
  const c = controller(); let calls = 0;
  const a = c.request({}, null, null, options(async () => { calls++; })), old = c.getSnapshot()!.id; c.cancel(old); await a;
  const b = c.request({}, null, null, options(async () => { calls++; })), current = c.getSnapshot()!.id;
  await c.execute(old); c.cancel(old); assert.equal(c.getSnapshot()!.id, current); assert.equal(calls, 0);
  await c.execute(current); assert.equal(await b, "completed"); assert.equal(calls, 1);
});
test("stale target or write protection prevents execution and requires a new review", async () => {
  const c = controller(); let calls = 0;
  const result = c.request({}, {}, {}, { ...options(async () => { calls++; }), validate: () => false });
  const id = c.getSnapshot()!.id; await c.execute(id);
  assert.equal(calls, 0); assert.equal(c.getSnapshot()!.phase, "failed"); assert.match(c.getSnapshot()!.error, /変更処理は開始していません/u);
  await c.execute(id); assert.equal(calls, 0); c.cancel(id); assert.equal(await result, "unconfirmed");
});
test("lost or failed response is retained, never automatically retried or called cancellation", async () => {
  const c = controller(); let calls = 0, errors = 0;
  const result = c.request({}, null, null, { ...options(async () => { calls++; throw Error("応答未確認"); }), onError: () => { errors++; } });
  const id = c.getSnapshot()!.id; await c.execute(id); await c.execute(id);
  assert.equal(calls, 1); assert.equal(errors, 1); assert.equal(c.getSnapshot()!.phase, "failed");
  c.cancel(id); assert.equal(await result, "unconfirmed");
});
test("disposed owner cancels pending decisions but does not cancel accepted native work", async () => {
  const c = controller(), owner = {}; let calls = 0, done!: () => void;
  const first = c.request(owner, {}, {}, options(async () => { calls++; })); c.disposeOwner(owner); assert.equal(await first, "cancelled"); assert.equal(calls, 0);
  const second = c.request(owner, {}, {}, options(async () => { calls++; await new Promise<void>(resolve => { done = resolve; }); }));
  const execution = c.execute(c.getSnapshot()!.id); c.disposeOwner(owner);
  assert.equal(c.getSnapshot()!.phase, "running"); done(); await execution; assert.equal(await second, "completed"); assert.equal(calls, 1);
});
test("closing a parent transfers the same active request without another action or identity", async () => {
  const c = controller(), parent = {}, opener = {};
  const result = c.request({}, parent, opener, options(async () => {})), before = c.getSnapshot()!;
  c.detachParent({}); assert.equal(c.getSnapshot()!.parent, parent);
  c.detachParent(parent); assert.equal(c.getSnapshot()!.id, before.id); assert.equal(c.getSnapshot()!.opener, opener); assert.equal(c.getSnapshot()!.parent, null);
  c.cancel(before.id); await result;
});
test("explicit progress cancellation is single, never accepts or dismisses the original work", async () => {
  const c = controller(); let actions = 0, cancels = 0, finish!: () => void, finishCancel!: () => void;
  const result = c.request({}, null, null, { ...options(async () => { actions++; await new Promise<void>(r => { finish = r; }); }), progressAction: { label: "停止要求", action: async () => { cancels++; await new Promise<void>(r => { finishCancel = r; }); } } });
  const id = c.getSnapshot()!.id;
  await c.runProgressAction(id); assert.equal(cancels, 0); assert.equal(actions, 0);
  const work = c.execute(id), cancellation = c.runProgressAction(id);
  await c.runProgressAction(id); c.cancel(id);
  assert.equal(cancels, 1); assert.equal(actions, 1); assert.equal(c.getSnapshot()!.phase, "running");
  finishCancel(); await cancellation; assert.equal(c.getSnapshot()!.progressDone, true);
  await c.runProgressAction(id); assert.equal(cancels, 1);
  finish(); await work; assert.equal(await result, "completed");
});
test("failed progress request retains awaited work and requires explicit progress retry", async () => {
  const c = controller(); let cancels = 0, finish!: () => void;
  const result = c.request({}, null, null, { ...options(async () => { await new Promise<void>(r => { finish = r; }); }), progressAction: { label: "停止要求", action: async () => { if (++cancels === 1) throw Error("停止要求の応答喪失"); } } });
  const id = c.getSnapshot()!.id, work = c.execute(id);
  await c.runProgressAction(id); assert.equal(cancels, 1); assert.equal(c.getSnapshot()!.phase, "running");
  assert.match(c.getSnapshot()!.progressError!, /応答喪失/u); assert.equal(c.getSnapshot()!.progressDone, undefined);
  await c.runProgressAction(id); assert.equal(cancels, 2); assert.equal(c.getSnapshot()!.progressDone, true);
  finish(); await work; await result;
});
test("late progress acknowledgement cannot change a completed or newer decision", async () => {
  const c = controller(); let finish!: () => void, finishCancel!: () => void;
  const first = c.request({}, null, null, { ...options(async () => { await new Promise<void>(r => { finish = r; }); }), progressAction: { label: "停止要求", action: async () => { await new Promise<void>(r => { finishCancel = r; }); } } });
  const id = c.getSnapshot()!.id, work = c.execute(id), cancellation = c.runProgressAction(id);
  finish(); await work; await first;
  const next = c.request({}, null, null, options(async () => {})), nextId = c.getSnapshot()!.id;
  finishCancel(); await cancellation; assert.equal(c.getSnapshot()!.id, nextId); assert.equal(c.getSnapshot()!.phase, "choosing");
  c.cancel(nextId); await next;
});
