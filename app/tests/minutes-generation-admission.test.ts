import test from "node:test";
import assert from "node:assert/strict";
import { MinutesAdmissionStore } from "../src/minutes-generation-admission.ts";

test("pending minutes admission survives view replacement and submits only once", async () => {
  const store = new MinutesAdmissionStore(() => "operation");
  let release!: () => void, calls = 0;
  const first = store.start("meeting", "会議", "fixed-input", async id => {
    assert.equal(id, "operation"); calls++;
    await new Promise<void>(resolve => { release = resolve; });
  });
  await Promise.resolve();
  assert.equal(store.getSnapshot()[0].pending, true);
  const second = store.start("meeting", "会議", "changed-input", async () => { calls++; });
  assert.equal(first, second); assert.equal(calls, 1);
  release(); await first;
  assert.deepEqual(store.getSnapshot(), []);
});

test("lost admission acknowledgement retains the original ID for an explicit retry", async () => {
  let ids = 0; const store = new MinutesAdmissionStore(() => `operation-${++ids}`);
  await assert.rejects(store.start("meeting", "会議", "fixed", async () => { throw Error("応答喪失"); }));
  assert.equal(store.getSnapshot()[0].pending, false);
  assert.match(store.getSnapshot()[0].error, /応答喪失/);
  await store.start("meeting", "会議", "fixed", async id => { assert.equal(id, "operation-1"); });
  assert.equal(ids, 1); assert.deepEqual(store.getSnapshot(), []);
});

test("admission errors remain observable after unmount and a committed job clears uncertainty", async () => {
  const store = new MinutesAdmissionStore(() => "operation");
  let changes = 0; const stop = store.subscribe(() => { changes++; });
  await assert.rejects(store.start("meeting", "会議", "fixed", async () => { stop(); throw Error("遅延したエラー"); }));
  assert.equal(changes, 1); assert.match(store.getSnapshot()[0].error, /遅延/);
  store.observe("other", ["operation"]); assert.equal(store.getSnapshot().length, 1);
  store.observe("meeting", ["operation"]); assert.deepEqual(store.getSnapshot(), []);
});
