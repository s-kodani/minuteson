import test from "node:test";
import assert from "node:assert/strict";
import { checkedStartupInstance } from "../src/startup-instance-state.ts";
const targetId = "11111111-1111-4111-8111-111111111111";
const valid = { schemaVersion: 1, supported: true, targetId, canRestart: true, message: "復旧できます" };
test("startup recovery only enables restart with a verified opaque target", () => {
  assert.deepEqual(checkedStartupInstance(valid), valid);
  assert.deepEqual(checkedStartupInstance({ ...valid, targetId: null, canRestart: false }).targetId, null);
  for (const value of [null, {}, { ...valid, schemaVersion: 2 }, { ...valid, targetId: "46296" }, { ...valid, targetId: null }, { ...valid, canRestart: false }, { ...valid, supported: false }, { ...valid, pid: 46296 }, { ...valid, message: "bad\0text" }, { ...valid, message: "音".repeat(2000) }]) assert.throws(() => checkedStartupInstance(value));
});
