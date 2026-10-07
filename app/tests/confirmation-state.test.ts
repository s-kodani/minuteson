import { test } from "node:test";
import assert from "node:assert/strict";
import { ConfirmationController } from "../src/confirmation-state.ts";
const options = (action: () => Promise<void>) => ({ title: "削除を確認", target: "固定した対象", message: "削除した入力は復元できません。", confirmLabel: "削除する", action });
const controller = () => new ConfirmationController<object, object>();

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
