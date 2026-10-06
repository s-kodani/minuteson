import { test } from "node:test";
import assert from "node:assert/strict";
import { outsideDialog } from "../src/modal-state.ts";

test("only actual backdrop coordinates count as outside, not content or padding", () => {
  const bounds = { left: 24, top: 32, right: 504, bottom: 432 };
  for (const [x, y] of [[24, 32], [504, 432], [24, 200], [300, 32], [300, 200]]) assert.equal(outsideDialog(x, y, bounds), false);
  for (const [x, y] of [[23, 200], [505, 200], [300, 31], [300, 433]]) assert.equal(outsideDialog(x, y, bounds), true);
});
