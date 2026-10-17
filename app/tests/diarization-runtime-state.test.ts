import test from "node:test";
import assert from "node:assert/strict";
import { acceptDiarizationRuntime, diarizationRuntimeProgress, type DiarizationRuntime } from "../src/diarization-runtime-state.ts";
import { jobProgress, progressPercent } from "../src/operation-progress-state.ts";
import type { AnalysisJob } from "../src/analysis-state.ts";

const job: AnalysisJob = { id: "job", kind: "diarization", state: "running", specHash: "hash", attempt: 1, completedUnits: 1, totalUnits: 2, errorCode: null, outputAnalysisId: null, pauseReason: null, retryOfJobId: null };
const runtime: DiarizationRuntime = { jobId: "job", specHash: "hash", attempt: 1, unitIndex: 1, invocationOrder: 2, sequence: 3, engineBasisPoints: 1250, finishing: false };
const render = (value = runtime, current = job) => diarizationRuntimeProgress(current, jobProgress(current, "会議A")!, value);

test("the bar displays the current engine unit percentage with a clear unit label", () => {
  const progress = render();
  assert.equal(progressPercent(progress.completed, progress.total), 12);
  assert.match(progress.detail!, /2 \/ 2 処理単位.*この処理単位のエンジン進捗/);
  assert.match(progress.label, /エンジンで処理中/);
  assert.equal(job.completedUnits, 1);
  assert.equal(progressPercent(render({ ...runtime, engineBasisPoints: 9999 }).completed, 10000), 99);
});

test("100 percent and postprocessing stay indeterminate until authoritative completion", () => {
  for (const value of [{ ...runtime, engineBasisPoints: 10000 }, { ...runtime, engineBasisPoints: 10000, finishing: true }]) {
    const progress = render(value);
    assert.equal(progress.total, null);
    assert.match(progress.label, /結果を照合・保存中/);
    assert.match(progress.detail!, /エンジン処理 100%/);
  }
  assert.equal(jobProgress({ ...job, state: "succeeded" }, "会議A"), null);
});

test("foreign, stale and paused telemetry cannot replace checkpoint progress", () => {
  for (const patch of [{ id: "other" }, { specHash: "new" }, { attempt: 2 }, { completedUnits: 2 }, { state: "paused" }, { state: "interrupted" }, { state: "pausing" }, { kind: "transcription" }, { totalUnits: null }]) {
    const current = { ...job, ...patch }, progress = jobProgress(current, "会議A")!;
    assert.equal(diarizationRuntimeProgress(current, progress, runtime), progress);
  }
});

test("malformed and reordered events cannot regress engine progress", () => {
  assert.deepEqual(acceptDiarizationRuntime(undefined, runtime), runtime);
  for (const patch of [{ sequence: 2 }, { sequence: 4, engineBasisPoints: 1200 }, { unitIndex: 0 }, { attempt: 0 }, { specHash: "other" }, { invocationOrder: 1 }, { engineBasisPoints: NaN }, { engineBasisPoints: 10001 }, { engineBasisPoints: -1 }, { engineBasisPoints: 0.5 }, { finishing: "yes" }, { unitIndex: Infinity }]) {
    assert.equal(acceptDiarizationRuntime(runtime, { ...runtime, ...patch }), runtime);
  }
  for (const value of [null, {}, "progress 10.00%"] ) assert.equal(acceptDiarizationRuntime(runtime, value), runtime);
  const finished = { ...runtime, sequence: 4, engineBasisPoints: 10000, finishing: true };
  assert.equal(acceptDiarizationRuntime(finished, { ...finished, sequence: 5, finishing: false }), finished);
  assert.equal(acceptDiarizationRuntime(runtime, { ...runtime, sequence: 4, engineBasisPoints: 2500 })?.engineBasisPoints, 2500);
});

test("resume and the next unit can restart their own engine percentage", () => {
  const next = { ...runtime, invocationOrder: 3, sequence: 1, engineBasisPoints: 0 };
  assert.deepEqual(acceptDiarizationRuntime(runtime, next), next);
  const nextUnit = { ...next, unitIndex: 2 };
  assert.deepEqual(acceptDiarizationRuntime(runtime, nextUnit), nextUnit);
  assert.equal(render(next).completed, 0);
});
