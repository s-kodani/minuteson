import test from "node:test";
import assert from "node:assert/strict";
import { engineConfigured, jobLabel } from "../src/analysis-state.ts";
test("unconfigured engine cannot start and states are distinct", () => {
  const engine = { executablePath: "/cli", modelPath: "/model", vadModelPath: null, threads: 4, useGpu: false };
  assert.ok(engineConfigured(engine));
  for (const patch of [{ modelPath: " " }, { executablePath: "" }, { threads: 0 }, { threads: 65 }, { threads: 1.5 }]) assert.equal(engineConfigured({ ...engine, ...patch }), false);
  assert.equal(jobLabel("pausing"), "停止処理中");
  assert.equal(jobLabel("interrupted"), "中断");
  assert.equal(jobLabel("other"), "other");
});
