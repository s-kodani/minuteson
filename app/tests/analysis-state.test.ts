import test from "node:test";
import assert from "node:assert/strict";
import { assertReviewedVersion, canRetryJob, engineConfigured, jobLabel, type VersionState, type AnalysisJob } from "../src/analysis-state.ts";

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

test("retry is distinct from resume and excludes unfinished descendants", () => {
  const job: AnalysisJob = { id: "source", kind: "transcription", state: "failed", completedUnits: 1, totalUnits: 3, errorCode: "engine_failed", outputAnalysisId: null, pauseReason: null, specHash: "a", retryOfJobId: null, attempt: 1 };
  assert.ok(canRetryJob(job, [job]));
  assert.ok(canRetryJob({ ...job, state: "cancelled" }, [job]));
  for (const state of ["queued", "running", "pausing", "paused", "interrupted", "succeeded"]) assert.equal(canRetryJob({ ...job, state }, [job]), false);
  assert.equal(canRetryJob({ ...job, kind: "minutes_generation" }, [job]), false);
  for (const state of ["queued", "running", "pausing", "paused", "interrupted"]) {
    const child = { ...job, id: "retry", retryOfJobId: job.id, attempt: 2, state };
    assert.equal(canRetryJob(job, [job, child]), false);
  }
  assert.ok(canRetryJob(job, [job, { ...job, id: "retry", retryOfJobId: job.id, state: "failed", attempt: 2 }]));
});
