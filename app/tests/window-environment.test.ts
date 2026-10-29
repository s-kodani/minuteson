import test from "node:test";
import assert from "node:assert/strict";
import { checkedWindowEnvironment, windowRole } from "../src/window-environment-state.ts";
test("window route permits only explicit roles", () => {
  assert.equal(windowRole(""), "main");
  assert.equal(windowRole("?window=recorder"), "recorder");
  assert.throws(() => windowRole("?window=unknown"));
});
test("native window identity and capability replies are validated before mounting UI", () => {
  const value = { schemaVersion: 1, role: "main", platform: "windows", capabilities: { voiceIdentity: true } };
  assert.deepEqual(checkedWindowEnvironment(value, "main"), value);
  for (const next of [{ ...value, role: "recorder" }, { ...value, platform: "linux" }, { ...value, capabilities: {} }, { ...value, schemaVersion: 2 }, { ...value, extra: true }]) assert.throws(() => checkedWindowEnvironment(next, "main"));
});
