import test from "node:test";
import assert from "node:assert/strict";
import { checkedWindowEnvironment, windowRole } from "../src/window-environment-state.ts";
test("window route permits only explicit roles", () => {
  assert.equal(windowRole(""), "main");
  assert.equal(windowRole("?window=recorder"), "recorder");
  assert.throws(() => windowRole("?window=unknown"));
});
test("native window identity and capability replies are validated before mounting UI", () => {
  const value = { schemaVersion: 1, productName: "MinutesOn", role: "main", platform: "windows", capabilities: { voiceIdentity: true } };
  assert.deepEqual(checkedWindowEnvironment(value, "main"), value);
  for (const next of [{ ...value, role: "recorder" }, { ...value, platform: "linux" }, { ...value, capabilities: {} }, { ...value, schemaVersion: 2 }, { ...value, extra: true }, { ...value, productName: undefined }, { ...value, productName: " " }, { ...value, productName: "MinutesOn\n" }, { ...value, productName: "M".repeat(257) }]) assert.throws(() => checkedWindowEnvironment(next, "main"));
  const qa = { ...value, productName: "MON QA" };
  assert.equal(checkedWindowEnvironment(qa, "main").productName, "MON QA");
});
