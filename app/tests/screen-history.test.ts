import test from "node:test";
import assert from "node:assert/strict";
import { ScreenHistory } from "../src/screen-history.ts";

test("back preserves the latest preparation and never adds a return loop", () => {
  const h = new ScreenHistory<{ title: string }>();
  h.visit("list", { title: "" }); h.visit("recording", { title: "initial" });
  h.visit("recording", { title: "edited" }); h.visit("settings", { title: "" });
  const target = h.previous!;
  assert.equal(target.value.title, "edited"); assert.equal(h.restore(target), true);
  h.visit("recording", target.value);
  assert.equal(h.previous!.key, "list"); assert.equal(h.restore(target), false);
});
test("history is bounded and distinguishes separate meeting screens", () => {
  const h = new ScreenHistory<number>();
  for (let i = 0; i < 40; i++) h.visit(`meeting-${i}`, i);
  let returns = 0;
  while (h.previous) { assert.equal(h.restore(h.previous), true); returns++; }
  assert.equal(returns, 32); assert.equal(h.current!.value, 7);
});
