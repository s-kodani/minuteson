import test from "node:test";
import assert from "node:assert/strict";
import { assertReviewedVersion, engineConfigured, jobLabel, type VersionState } from "../src/analysis-state.ts";

const state: VersionState = { meetingLockVersion: 2, currentId: "current", previousId: "previous", draft: null, versions: [
  { id: "candidate", revisionNo: 3, contentHash: "c", publicationState: "candidate", utteranceCount: 1, createdAt: 0, profile: {}, config: {} },
  { id: "previous", revisionNo: 1, contentHash: "p", publicationState: "committed", utteranceCount: 1, createdAt: 0, profile: {}, config: {} },
] };
test("only the exact reviewed candidate or previous head can be applied", () => {
  assert.doesNotThrow(() => assertReviewedVersion(state, { id: "candidate", contentHash: "c" }, false));
  assert.doesNotThrow(() => assertReviewedVersion(state, { id: "previous", contentHash: "p" }, true));
  assert.throws(() => assertReviewedVersion(state, { id: "candidate", contentHash: "changed" }, false));
  assert.throws(() => assertReviewedVersion(state, { id: "missing", contentHash: "c" }, false));
  assert.throws(() => assertReviewedVersion(state, { id: "previous", contentHash: "p" }, false));
  assert.throws(() => assertReviewedVersion(state, { id: "candidate", contentHash: "c" }, true));
});
test("unconfigured engine cannot start and states are distinct", () => {
  const engine = { executablePath: "/cli", modelPath: "/model", vadModelPath: null, threads: 4, useGpu: false };
  assert.ok(engineConfigured(engine));
  for (const patch of [{ modelPath: " " }, { executablePath: "" }, { threads: 0 }, { threads: 65 }, { threads: 1.5 }]) assert.equal(engineConfigured({ ...engine, ...patch }), false);
  assert.equal(jobLabel("pausing"), "停止処理中");
  assert.equal(jobLabel("interrupted"), "中断");
  assert.equal(jobLabel("other"), "other");
});
