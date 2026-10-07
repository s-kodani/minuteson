import { test } from "node:test";
import assert from "node:assert/strict";
import { ToastController } from "../src/toast-state.ts";
import { exportNoticeLevel } from "../src/minutes-state.ts";

test("an old component cleanup cannot detach a newer component's action", async () => {
  // Important avoids introducing real timers into this ownership-only test.
  const c = new ToastController(); let calls = 0;
  const old = () => { calls += 100; }, current = () => { calls++; };
  c.show({ key: "same", message: "保持している問題", level: "error", action: { kind: "retry", label: "再試行", run: old } });
  c.show({ key: "same", message: "保持している問題", level: "error", action: { kind: "retry", label: "再試行", run: current } });
  c.detachAction("same", old); await c.act("same", c.getSnapshot()[0].id); assert.equal(calls, 1);
  c.detachAction("same", current); assert.equal(c.getSnapshot()[0].action, undefined);
  await c.act("same", c.getSnapshot()[0].id); assert.equal(calls, 1); assert.equal(c.getSnapshot().length, 1);
});
test("Markdown only reports persisted success as success; cancelled is information and failed/unknown outcomes are important", () => {
  assert.equal(exportNoticeLevel("succeeded"), "success"); assert.equal(exportNoticeLevel(null), "info");
  for (const state of ["writing", "failed", "interrupted"] as const) assert.equal(exportNoticeLevel(state), "error");
});
