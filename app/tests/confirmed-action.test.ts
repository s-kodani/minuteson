import { test } from "node:test";
import assert from "node:assert/strict";
import { SavedEditsCancelled, withConfirmedEdits } from "../src/confirmed-action.ts";
test("confirmed action waits for saved edits and runs once", async () => {
  const events: string[] = [];
  await withConfirmedEdits(async action => { events.push("saved"); await action(); }, async () => { events.push("action"); });
  assert.deepEqual(events, ["saved", "action"]);
});
test("swallowed confirmed action failure remains a failure without retry", async () => {
  let calls = 0;
  await assert.rejects(withConfirmedEdits(async action => { try { await action(); } catch {} }, async () => { calls++; throw Error("応答喪失"); }), /応答喪失/u);
  assert.equal(calls, 1);
});
test("operation failure is reported only by the confirmed caller, not the save barrier", async () => {
  const cause = Error("operation failed"); let saveReports = 0, calls = 0;
  await assert.rejects(withConfirmedEdits(async action => {
    try { await action(); } catch { saveReports++; }
  }, async () => { calls++; throw cause; }), e => e === cause);
  assert.equal(saveReports, 0);
  assert.equal(calls, 1);
});
test("save failure which skips action is not reported as completion", async () => {
  let calls = 0;
  await assert.rejects(withConfirmedEdits(async () => {}, async () => { calls++; }), /変更処理は開始していません/u);
  assert.equal(calls, 0);
});
test("explicit edit cancellation never executes or becomes an unconfirmed save failure", async () => {
  let calls = 0;
  await assert.rejects(withConfirmedEdits(async () => "cancelled", async () => { calls++; }), SavedEditsCancelled);
  assert.equal(calls, 0);
});
test("propagated save failure stays original and repeated callbacks do not replay", async () => {
  const cause = Error("save failed"); let calls = 0;
  await assert.rejects(withConfirmedEdits(async () => { throw cause; }, async () => { calls++; }), e => e === cause);
  await assert.rejects(withConfirmedEdits(async action => { await action(); await action(); }, async () => { calls++; }), /既に開始/u);
  assert.equal(calls, 1);
});
