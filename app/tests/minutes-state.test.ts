import test from "node:test";
import assert from "node:assert/strict";
import { minutesConfigured, type MinutesEngine } from "../src/minutes-state.ts";
const engine: MinutesEngine = { executablePath: "/engine/llama-completion", modelPath: "/model.gguf", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 };
test("minutes engine requires integral bounded parameters and context for the fixed input", () => {
  assert.ok(minutesConfigured(engine));
  for (const patch of [{ modelPath: " " }, { executablePath: "" }, { threads: 0 }, { threads: 1.5 }, { contextTokens: 32769 }, { predictionTokens: 8192 }, { gpuLayers: 257 }, { seed: -1 }, { promptBudget: 6000 }, { promptBudget: 2047 }]) assert.equal(minutesConfigured({ ...engine, ...patch }), false);
});
