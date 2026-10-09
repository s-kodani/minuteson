import test from "node:test";
import assert from "node:assert/strict";
import { checkedOperations, emptyOperations, mergeOperations, relayOperation, type OperationSnapshot } from "../src/recorder-operation-state.ts";
const errorState = (message = "開けません"): OperationSnapshot => { const s = emptyOperations(); s.entries[0] = { source: "window_action", revision: 1, eventId: "window_action:1", message, dismissed: false }; return s; };
test("operation snapshots require complete fixed sources and exact identities", () => {
  assert.deepEqual(checkedOperations(emptyOperations()), emptyOperations());
  assert.deepEqual(checkedOperations(errorState()), errorState());
  for (const input of [{ ...emptyOperations(), entries: [] }, { ...emptyOperations(), foreign: true }, { ...emptyOperations(), entries: Array(4).fill(emptyOperations().entries[0]) }]) assert.throws(() => checkedOperations(input));
});
test("forged event ids, unsafe revisions and cleared-but-dismissed rows fail closed", () => {
  for (const change of [{ eventId: "confirmation:1" }, { revision: NaN }, { revision: 0 }, { revision: Number.MAX_SAFE_INTEGER + 1 }, { eventId: "window_action:999" }, { message: " " }, { message: "x".repeat(8193) }, { message: "\0" }]) {
    const s = errorState(); Object.assign(s.entries[0], change); assert.throws(() => checkedOperations(s));
  }
  const clear = emptyOperations(); clear.entries[0].dismissed = true; assert.throws(() => checkedOperations(clear));
});
test("older rows cannot overwrite retained state and same-version contradictions reject", () => {
  assert.deepEqual(mergeOperations(errorState(), emptyOperations()), errorState());
  assert.throws(() => mergeOperations(errorState(), errorState("別の失敗")), /矛盾/u);
  const reordered = { entries: errorState().entries.map(e => Object.fromEntries(Object.entries(e).reverse())), schemaVersion: 1 }; assert.deepEqual(mergeOperations(errorState(), reordered), errorState());
});
test("toast dismissal preserves cause and event identity", () => {
  const s = errorState(); s.entries[0].revision = 2; s.entries[0].dismissed = true;
  const next = mergeOperations(errorState(), s); assert.equal(next.entries[0].message, "開けません"); assert.equal(next.entries[0].eventId, "window_action:1"); assert.equal(next.entries[0].dismissed, true);
});
test("relay uses the observed source revision and checks the exact accepted message", async () => {
  const calls: unknown[] = []; let state = emptyOperations();
  const result = await relayOperation({ read: async () => state, publish: async (source, revision, message) => { calls.push([source, revision, message]); state = errorState(); return state; } }, "window_action", "開けません");
  assert.deepEqual(result, errorState()); assert.deepEqual(calls, [["window_action", 0, "開けません"]]);
});
test("lost relay response is read back without repeating the publish or native action", async () => {
  let state = emptyOperations(), writes = 0;
  const result = await relayOperation({ read: async () => state, publish: async () => { writes++; state = errorState(); throw Error("response lost"); } }, "window_action", "開けません");
  assert.equal(writes, 1); assert.deepEqual(result, errorState());
});
test("newer cause defeats stale clear without automatic replay", async () => {
  let reads = 0, writes = 0;
  await assert.rejects(relayOperation({ read: async () => ++reads === 1 ? errorState() : errorState("新しい失敗"), publish: async () => { writes++; throw Error("stale"); } }, "window_action", null), /共有状態/u);
  assert.equal(writes, 1);
});
test("disposed publisher does not send metadata and unchanged dismissed cause stays dismissed", async () => {
  let writes = 0; const bridge = { read: async () => errorState(), publish: async () => { writes++; return errorState(); } };
  assert.equal(await relayOperation(bridge, "window_action", null, () => false), null); assert.equal(writes, 0);
  const dismissed = errorState(); dismissed.entries[0].dismissed = true; dismissed.entries[0].revision = 2;
  assert.deepEqual(await relayOperation({ ...bridge, read: async () => dismissed }, "window_action", "開けません"), dismissed); assert.equal(writes, 0);
});
