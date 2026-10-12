import test from "node:test";
import assert from "node:assert/strict";
import { JobNotificationTracker, assertNotificationJobs, verifiedJobOutput, type NotificationJob } from "../src/job-notification-state.ts";
const job = (patch: Partial<NotificationJob> = {}): NotificationJob => ({ id: "job", kind: "transcription", meetingId: "meeting", meetingTitle: "会議A", operationId: "operation", inputHash: "b".repeat(64), specHash: "a".repeat(64), state: "queued", createdAt: 100, updatedAt: 100, attempt: 1, retryOfJobId: null, completedUnits: 0, totalUnits: 2, errorCode: null, pauseReason: null, outputAnalysisId: null, outputMinutesId: null, ...patch });
test("direct Markdown success needs an exact receipt and never announces a candidate",()=>{
  const tracker=new JobNotificationTracker(50);tracker.ingest([job({kind:"minutes_generation"})]);
  const done=job({kind:"minutes_generation",state:"succeeded",completedUnits:2,updatedAt:102});tracker.ingest([done]);
  const receipt={job:done,contentHash:"c".repeat(64),publicationState:"markdown"};
  const verified=verifiedJobOutput(done,receipt);assert.equal(verified,"markdown");
  const notice=tracker.confirmOutput(done,verified);assert.match(notice!.message,/議事録を保存/);assert.doesNotMatch(notice!.message,/候補|採用/);
});
test("superseded Markdown generation is acknowledged silently without replacing current human editing",()=>{
  const tracker=new JobNotificationTracker(50);tracker.ingest([job({kind:"minutes_generation"})]);
  const done=job({kind:"minutes_generation",state:"succeeded",completedUnits:2,updatedAt:102});tracker.ingest([done]);
  assert.equal(tracker.confirmOutput(done,verifiedJobOutput(done,{job:done,contentHash:"c".repeat(64),publicationState:"superseded"})),null);
  assert.equal(tracker.ingest([done]).pending.length,0);
});
test("Markdown receipt cannot pass for another kind, incomplete units or a revision candidate",()=>{
  for(const done of [job({state:"succeeded",completedUnits:2}),job({kind:"minutes_generation",state:"succeeded",completedUnits:1}),job({kind:"minutes_generation",state:"succeeded",completedUnits:2,outputMinutesId:"candidate"})]) {
    assert.throws(()=>verifiedJobOutput(done,{job:done,contentHash:"c".repeat(64),publicationState:"markdown"}));
  }
});
test("startup and restored historical success are silent, with retained latest failed state", () => {
  const tracker = new JobNotificationTracker(200);
  const state = tracker.ingest([job({ state: "failed", errorCode: "engine_failed" })]);
  assert.equal(state.events.length, 0); assert.equal(state.issues.length, 1);
  assert.equal(tracker.ingest([job({ id: "restored", state: "succeeded", outputAnalysisId: "saved", createdAt: 150, updatedAt: 300 })]).pending.length, 0);
});
test("receipt admission, progress and candidate publication are separate", () => {
  const tracker = new JobNotificationTracker(50); tracker.ingest([]);
  assert.match(tracker.ingest([job()]).events[0].message, /受け付け.*完了は後/);
  assert.equal(tracker.ingest([job({ state: "running", updatedAt: 101 })]).events.length, 0);
  const completed = job({ state: "succeeded", completedUnits: 2, updatedAt: 102, outputAnalysisId: "candidate" });
  const state = tracker.ingest([completed]); assert.equal(state.events.length, 0); assert.equal(state.pending.length, 1);
  assert.deepEqual(tracker.watchedIds(), [completed.id]);
  assert.match(tracker.confirmOutput(completed, "candidate")!.message, /候補を保存.*内容確認後/);
  assert.equal(tracker.confirmOutput(completed, "candidate"), null);
  assert.equal(tracker.ingest([completed]).pending.length, 0);
});
test("failed proof stays pending and missing output cannot announce success", () => {
  const tracker = new JobNotificationTracker(50); tracker.ingest([job()]);
  const completed = job({ state: "succeeded", updatedAt: 102 }); tracker.ingest([completed]);
  assert.throws(() => tracker.confirmOutput(completed, "candidate"));
  assert.equal(tracker.ingest([completed]).pending.length, 1);
});
test("a new latest retry resolves prior failure without erasing its historical job", () => {
  const tracker = new JobNotificationTracker(50); tracker.ingest([job()]);
  const failed = job({ state: "failed", updatedAt: 101, errorCode: "engine_failed" });
  const state = tracker.ingest([failed]); assert.equal(state.events[0].important, true);
  assert.equal(state.issues.length, 1); assert.equal(tracker.ingest([failed]).events.length, 0);
  const retry = job({ id: "retry", operationId: "retry-op", createdAt: 102, updatedAt: 102, attempt: 2, retryOfJobId: failed.id });
  const retried = tracker.ingest([retry, failed]); assert.equal(retried.issues.length, 0); assert.equal(retried.healthyGroups.length, 1);
});
test("different newer work cannot hide a watched failure in the same meeting and kind", () => {
  const tracker = new JobNotificationTracker(50); tracker.ingest([job()]);
  const newer = job({ id: "newer", operationId: "different-work", createdAt: 110, updatedAt: 110 });
  tracker.ingest([newer, job()]);
  const failed = job({ state: "failed", errorCode: "engine_failed", updatedAt: 120 });
  const state = tracker.ingest([newer, failed]);
  assert.equal(state.events[0].topic, "issue"); assert.equal(state.issues[0].id, failed.id);
  assert.ok(tracker.watchedIds().includes(failed.id));
});
test("same-operation terminal cancellation and checkpoint pause, not requests, are notified", () => {
  const tracker = new JobNotificationTracker(50); tracker.ingest([job()]);
  assert.equal(tracker.ingest([job({ state: "pausing", updatedAt: 101 })]).events.length, 0);
  assert.match(tracker.ingest([job({ state: "paused", updatedAt: 102, pauseReason: "recording" })]).events[0].message, /録音・取り込み優先/);
  assert.match(tracker.ingest([job({ state: "queued", updatedAt: 103 })]).events[0].message, /再開を受け付け/);
  assert.match(tracker.ingest([job({ state: "cancelled", updatedAt: 104 })]).events[0].message, /取り消し.*現在版は保持/);
});
test("stale progress does not replace newer state; malformed and changed identities are rejected atomically", () => {
  const tracker = new JobNotificationTracker(50); tracker.ingest([job({ state: "running", updatedAt: 110 })]);
  assert.equal(tracker.ingest([job({ updatedAt: 105 })]).events.length, 0);
  assert.throws(() => tracker.ingest([job({ operationId: "different" })]));
  for (const patch of [{ state: "invented" }, { totalUnits: -1 }, { attempt: 0 }, { specHash: "wrong" }, { outputMinutesId: undefined }]) assert.throws(() => assertNotificationJobs([job(patch as never)]));
  assert.throws(() => assertNotificationJobs([job(), job()]));
});
test("output proof binds exact meeting, job, operation, fixed input and saved version", () => {
  for (const kind of ["transcription", "diarization", "minutes_generation"] as const) {
    const completed = job({ kind, state: "succeeded", outputAnalysisId: kind === "minutes_generation" ? null : "saved", outputMinutesId: kind === "minutes_generation" ? "saved" : null });
    const proof = { job: completed, contentHash: "c".repeat(64), publicationState: "candidate" };
    assert.equal(verifiedJobOutput(completed, proof), "candidate");
    assert.equal(verifiedJobOutput(completed, { ...proof, publicationState: "committed" }), "committed");
    for (const patch of [{ id: "other" }, { meetingId: "other" }, { kind: "transcript_cleanup" }, { operationId: "other" }, { inputHash: "d".repeat(64) }, { specHash: "e".repeat(64) }, { state: "running" }, { outputAnalysisId: "other", outputMinutesId: "other" }]) assert.throws(() => verifiedJobOutput(completed, { ...proof, job: { ...completed, ...patch } }));
    for (const patch of [{ contentHash: "not-a-hash" }, { publicationState: "internal_snapshot" }, { job: null }]) assert.throws(() => verifiedJobOutput(completed, { ...proof, ...patch }));
  }
});
test("stale completion replies cannot regress proven outputs; deleted jobs drop watches", () => {
  const tracker = new JobNotificationTracker(50); tracker.ingest([job()]);
  const saved = job({ state: "succeeded", outputAnalysisId: "saved", updatedAt: 110 });
  tracker.ingest([saved]); assert.equal(tracker.ingest([job({ updatedAt: 105 })]).pending.length, 1);
  assert.throws(() => tracker.ingest([job({ state: "cancelled", updatedAt: 111 })]));
  tracker.ingest([]); assert.deepEqual(tracker.watchedIds(), []);
});
