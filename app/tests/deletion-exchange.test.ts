import test from "node:test";
import assert from "node:assert/strict";
import { DeletionExchange } from "../src/deletion-exchange.ts";
import type { DataRequest, DataSnapshot } from "../src/data-operation-state.ts";
const request = { token: "owned", confirmationId: "confirmation" } as DataRequest;
const snapshot = (next: object) => ({ token: "owned", revision: 5, busy: false, phase: "ready", ...next }) as DataSnapshot;
for (const code of ["meeting_deletion_not_started", "audio_deletion_not_started", "model_deletion_not_started"]) {
  test(`${code}: lost submit responses retain ownership until a newer matching receipt`, async () => {
    const exchange = new DeletionExchange(code);
    await assert.rejects(() => exchange.send(request, 4, async () => { throw Error("lost"); }));
    assert.equal(exchange.getSnapshot().unknown, true);
    assert.equal(exchange.reconcile(snapshot({ token: "another", phase: "completed" }), undefined, () => false), false);
    assert.equal(exchange.reconcile(snapshot({ revision: 4, phase: "completed" }), undefined, () => false), false);
    assert.equal(exchange.reconcile(snapshot({ phase: "completed" }), undefined, () => false), true);
    assert.equal(exchange.pending.current, null);
    assert.equal(exchange.getSnapshot().unknown, false);
  });
  test(`${code}: a typed refusal never becomes an unknown accepted operation`, async () => {
    const exchange = new DeletionExchange(code);
    await assert.rejects(() => exchange.send(request, 4, async () => { throw { code, message: "not accepted", operationId: null, retryDisposition: "not_started" }; }), /not accepted/);
    assert.equal(exchange.pending.current, null); assert.equal(exchange.getSnapshot().unknown, false);
    await assert.rejects(() => exchange.review({ id: "meeting", pending: false, revision: 4 }, async () => { throw {code, message:"not accepted", operationId:null, retryDisposition:"not_started"}; }), /not accepted/);
    assert.equal(exchange.choice.current, null);
  });
}
test("concurrent commands coalesce and release the operation lock after failure", async () => {
  const exchange = new DeletionExchange("meeting_deletion_not_started");
  let finish!: () => void, called = 0;
  const first = exchange.run(() => new Promise(resolve => { called++; finish = resolve; }));
  await exchange.run(async () => { called++; }); assert.equal(called, 1);
  finish(); await first; assert.equal(exchange.getSnapshot().rpcBusy, false);
  await exchange.run(async () => { throw { code: "failed", message: "safe message", retryDisposition: "never" }; });
  assert.equal(exchange.getSnapshot().error, "safe message"); assert.equal(exchange.running.current, false);
});
