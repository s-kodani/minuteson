import test from "node:test";
import assert from "node:assert/strict";
import { assertJobModelPreview, assertJobModelResult, canRebindJob, rebindingEntries, ReviewedModelRetry, type JobModelPreview } from "../src/model-rebinding-state.ts";
import { rebindingFixture, meetingId, operationId } from "./model-rebinding-fixture.ts";

test("native three-stage retries include restored preparation interruptions but exclude ordinary pause, invalid progress and unfinished children", () => {
  for (const kind of ["transcription", "diarization", "minutes_generation"]) {
    const { job, result } = rebindingFixture(kind);
    assert.ok(canRebindJob(job, [job]));
    for (const state of ["failed", "cancelled"]) assert.ok(canRebindJob({ ...job, state }, []));
    for (const patch of [{ state: "paused" }, { state: "succeeded" }, { errorCode: "engine_failed" }, { kind: "audio_import" }, { attempt: 1000 }, { attempt: 0 }, { completedUnits: -1 }, { completedUnits: 4 }, { totalUnits: null }, { totalUnits: 0 }, { id: "bad" }, { specHash: "bad" }]) assert.equal(canRebindJob({ ...job, ...patch }, []), false);
    for (const state of ["queued", "running", "pausing", "paused", "interrupted"]) assert.equal(canRebindJob(job, [{ ...result, state }]), false);
    assert.ok(canRebindJob(job, [{ ...result, state: "succeeded" }]));
  }
});
test("destination catalog requires explicit successful CPU evidence for the same engine and rejects duplicates", () => {
  const a = rebindingFixture(), b = rebindingFixture("diarization");
  assert.deepEqual(rebindingEntries([a.entry, b.entry], a.job.kind), [a.entry]);
  assert.deepEqual(rebindingEntries([{ ...a.entry, compatibility: null }], a.job.kind), []);
  assert.throws(() => rebindingEntries([a.entry, a.entry], a.job.kind));
  assert.throws(() => rebindingEntries([{ ...a.entry, compatibility: { ...a.entry.compatibility!, packageId: operationId } }], a.job.kind));
});
test("review validates fixed source, path-free conditions, selected package, CPU operation and all dependencies for three engines", () => {
  for (const kind of ["transcription", "diarization", "minutes_generation"]) {
    const { preview, job, entry } = rebindingFixture(kind);
    assert.doesNotThrow(() => assertJobModelPreview(preview, meetingId, job, entry));
    const mutations: ((p: JobModelPreview) => void)[] = [
      p => { p.schemaVersion = 2 as 1; }, p => { p.source.specHash = "0".repeat(64); }, p => { p.source.meetingId = operationId; },
      p => { p.source.completedUnits = 0; }, p => { p.source.attempt = 2; }, p => { p.reviewHash = "bad"; }, p => { p.checkpointHash = "bad"; },
      p => { p.logicalConditions.engine = "wrong"; }, p => { p.logicalConditions.configuration.executablePath = 1; }, p => { p.logicalConditions.configuration.threads = 0; },
      p => { p.binding.schemaVersion = 2 as 1; }, p => { p.binding.package.packageId = operationId; }, p => { p.binding.package.manifest.modelVersion = "2"; },
      p => { p.binding.compatibility.operationId = operationId; }, p => { p.binding.compatibility.reviewHash = "0".repeat(64); },
      p => { p.binding.files.pop(); }, p => { p.binding.files.at(-1)!.sha256 = "0".repeat(64); }, p => { p.binding.files[0].path = "relative"; },
      p => { p.binding.package.files[0].actualSize++; }, p => { p.binding.package.files[0].role = "unknown"; }, p => { p.binding.package.manifest.files[0].expectedHash = null; },
    ];
    for (const mutate of mutations) { const p = structuredClone(preview); mutate(p); assert.throws(() => assertJobModelPreview(p, meetingId, job, entry)); }
    if (kind === "transcription") { const p = structuredClone(preview); p.logicalConditions.vocabularyHint = null; assert.throws(() => assertJobModelPreview(p, meetingId, job, entry)); }
    if (kind === "diarization") for (const count of [0, 101, -1]) { const p = structuredClone(preview); p.logicalConditions.trackSettings![0].numSpeakers = count; assert.throws(() => assertJobModelPreview(p, meetingId, job, entry)); }
  }
});
test("receipts allow progressed jobs but bind operation source attempt input and completion lineage", () => {
  const { preview, result } = rebindingFixture();
  assert.doesNotThrow(() => assertJobModelResult(result, operationId, preview));
  assert.doesNotThrow(() => assertJobModelResult({ ...result, state: "succeeded", completedUnits: 3 }, operationId, preview));
  for (const patch of [{ operationId: preview.source.operationId }, { id: preview.source.id }, { meetingId: operationId }, { kind: "diarization" }, { retryOfJobId: null }, { attempt: 3 }, { completedUnits: 0 }, { totalUnits: 4 }, { inputHash: "0".repeat(64) }]) assert.throws(() => assertJobModelResult({ ...result, ...patch }, operationId, preview));
});
test("lost replies and cancellation retain exact operation and immutable review for read-only receipts or same-operation retries", () => {
  const { preview } = rebindingFixture();
  const attempt = new ReviewedModelRetry(preview, operationId);
  preview.source.completedUnits = 0;
  assert.equal(attempt.sent, false);
  const request = attempt.request(); assert.equal(attempt.sent, true);
  request.preview.source.completedUnits = 99;
  assert.equal(attempt.receipt().preview.source.completedUnits, 1);
  assert.deepEqual(attempt.request(), { token: operationId, preview: attempt.receipt().preview });
  attempt.cancelled = true;
  assert.equal(attempt.receipt().operationId, operationId);
  assert.throws(() => new ReviewedModelRetry(preview, "bad"));
});
