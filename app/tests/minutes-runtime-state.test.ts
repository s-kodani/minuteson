import test from "node:test";
import assert from "node:assert/strict";
import { acceptMinutesRuntime, minutesRuntimeDetail, type MinutesRuntime } from "../src/minutes-runtime-state.ts";
import { jobProgress, progressPercent } from "../src/operation-progress-state.ts";
import type { AnalysisJob } from "../src/analysis-state.ts";
const job: AnalysisJob = { id: "job", specHash: "hash", attempt: 1, state: "running", kind: "minutes_generation", completedUnits: 2, totalUnits: 10, errorCode: null, outputAnalysisId: null, pauseReason: null, retryOfJobId: null };
const runtime: MinutesRuntime = { jobId: "job", specHash: "hash", attempt: 1, unitIndex: 2, invocation: "unit-a", invocationOrder: 3, startedAtUtcMs: 100, sequence: 3, phase: "input", merge: false, elapsedMs: 320000 };
test("current unit phases never display per-unit elapsed time or invent completed progress", () => {
  assert.equal(minutesRuntimeDetail(job,runtime), "3工程目：入力を解析中");
  assert.match(minutesRuntimeDetail(job,{...runtime,phase:"generating"})!, /本文を生成中/);
  assert.match(minutesRuntimeDetail(job,{...runtime,backend:"metal"})!, /GPU · 入力を解析中/);
  assert.match(minutesRuntimeDetail(job,{...runtime,backend:"cpu"})!, /CPU · 入力を解析中/);
  assert.match(minutesRuntimeDetail(job,{...runtime,phase:"generating",merge:true})!, /下書きを統合中/);
  const p=jobProgress(job,"会議")!; assert.equal(progressPercent(p.completed,p.total),20);
  for (const changed of [{state:"paused"},{state:"succeeded"},{attempt:2},{id:"other"},{specHash:"new"},{completedUnits:3}]) assert.equal(minutesRuntimeDetail({...job,...changed},runtime),undefined);
});
test("stale attempts, stages, invocations and malformed observations cannot replace newer progress", () => {
  assert.deepEqual(acceptMinutesRuntime(undefined,runtime),runtime);
  for (const v of [null, {}, {...runtime,elapsedMs:NaN},{...runtime,unitIndex:-1},{...runtime,merge:"true"},{...runtime,phase:"done"}]) assert.equal(acceptMinutesRuntime(runtime,v),runtime);
  for (const v of [{...runtime,sequence:2},{...runtime,sequence:4,phase:"starting"},{...runtime,unitIndex:1},{...runtime,invocation:"old",invocationOrder:2,startedAtUtcMs:99}]) assert.equal(acceptMinutesRuntime(runtime,v),runtime);
  assert.equal(acceptMinutesRuntime(runtime,{...runtime,sequence:4,phase:"generating"})?.phase,"generating");
  assert.equal(acceptMinutesRuntime(runtime,{...runtime,invocation:"resumed",invocationOrder:4,startedAtUtcMs:100,sequence:1,phase:"verifying",elapsedMs:0})?.invocation,"resumed");
  // An accepted event from another attempt still cannot describe this job.
  assert.equal(minutesRuntimeDetail(job,acceptMinutesRuntime(runtime,{...runtime,attempt:2})),undefined);
});
