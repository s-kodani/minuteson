import test from "node:test";
import assert from "node:assert/strict";
import { assertMinutesReview, assertMinutesVersion, minutesConfigured, type MinutesEngine, type MinutesVersions } from "../src/minutes-state.ts";

const engine: MinutesEngine = { executablePath: "/engine/llama-completion", modelPath: "/model.gguf", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 };
const state: MinutesVersions = { meetingLockVersion: 1, currentId: "current", previousId: "previous", draft: { id: "draft", lockVersion: 3 }, versions: [
  { id: "candidate", revisionNo: 3, contentHash: "hash", publicationState: "candidate", inputAnalysisId: "fixed", inputUpdated: false, itemCount: 2, createdAt: 0, profile: {} },
  { id: "previous", revisionNo: 1, contentHash: "previous-hash", publicationState: "committed", inputAnalysisId: "old-fixed", inputUpdated: true, itemCount: 1, createdAt: 0, profile: {} },
] };
test("minutes engine requires integral bounded parameters and context for the fixed input", () => {
  assert.ok(minutesConfigured(engine));
  for (const patch of [{ modelPath: " " }, { executablePath: "" }, { threads: 0 }, { threads: 1.5 }, { contextTokens: 32769 }, { predictionTokens: 8192 }, { gpuLayers: 257 }, { seed: -1 }, { promptBudget: 6000 }, { promptBudget: 2047 }]) assert.equal(minutesConfigured({ ...engine, ...patch }), false);
});
test("minutes review rejects replacement of the reviewed candidate, head, draft or input status", () => {
  const reviewed = state.versions[0];
  assert.doesNotThrow(() => assertMinutesReview(state, state, reviewed, false));
  assert.doesNotThrow(() => assertMinutesVersion(state, state.versions[1], true));
  assert.throws(() => assertMinutesVersion(state, reviewed, true));
  assert.throws(() => assertMinutesVersion(state, { ...reviewed, contentHash: "other" }, false));
  for (const patch of [{ meetingLockVersion: 2 }, { currentId: "new" }, { previousId: null }, { draft: null }, { draft: { id: "new-draft", lockVersion: 3 } }, { draft: { id: "draft", lockVersion: 4 } }, { versions: [{ ...reviewed, inputUpdated: true }] }]) assert.throws(() => assertMinutesReview({ ...state, ...patch }, state, reviewed, false));
});
