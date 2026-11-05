import test from "node:test";
import assert from "node:assert/strict";
import { jobLabel } from "../src/analysis-state.ts";
test("analysis job states have distinct labels", () => {
  assert.equal(jobLabel("pausing"), "停止処理中");
  assert.equal(jobLabel("interrupted"), "中断");
  assert.equal(jobLabel("other"), "other");
});
