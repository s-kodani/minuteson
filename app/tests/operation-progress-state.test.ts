import test from "node:test";
import assert from "node:assert/strict";
import { acceptTransferProgress, importProgress, jobProgress, modelProgress, operationElapsed, orderedProgress, progressPercent, ProgressRegistry, recordingSaveProgress } from "../src/operation-progress-state.ts";
import { acceptImport, type ImportSnapshot } from "../src/import-state.ts";
import { JobNotificationTracker, type NotificationJob } from "../src/job-notification-state.ts";
import type { RecordingSnapshot } from "../src/session.ts";

test("recording save is one indeterminate job until saving really finishes", () => {
  const snapshot: RecordingSnapshot = { revision: 1, phase: "stopping", sessionId: "session", meetingId: "meeting", elapsedMs: 1000, microphoneConnected: false, systemAudioConnected: false, errorMessage: null };
  const stopping = recordingSaveProgress(snapshot, "定例会議")!;
  const saving = recordingSaveProgress({ ...snapshot, revision: 2, phase: "saving" }, "定例会議")!;
  assert.equal(stopping.id, saving.id);
  assert.equal(saving.kind, "recording_save");
  assert.equal(saving.meetingId, "meeting");
  assert.equal(saving.target, "定例会議");
  assert.equal(saving.total, null);
  assert.equal(saving.completed, null);
  for (const phase of ["idle", "preparing", "recording", "error"] as const) assert.equal(recordingSaveProgress({ ...snapshot, phase }), null);
  assert.equal(recordingSaveProgress(null), null);
  assert.equal(recordingSaveProgress({ ...snapshot, sessionId: null }), null);
});

test("recording save shows measured WAV frames and stays active until saving finishes", () => {
  const snapshot: RecordingSnapshot = { revision: 1, phase: "saving", sessionId: "session", meetingId: "meeting", elapsedMs: 1000, microphoneConnected: false, systemAudioConnected: false, errorMessage: null };
  for (const completed of [0, 8192, 16000]) {
    const saving = recordingSaveProgress({ ...snapshot, progress: { stage: "mixing_audio", completed, total: 16000, unit: "frames" } })!;
    assert.equal(saving.label, "会議音声を生成中");
    assert.equal(saving.detail, `${completed.toLocaleString("ja-JP")} / 16,000 フレーム`);
    assert.equal(saving.state, "running");
    assert.equal(saving.completed, null);
    assert.equal(saving.total, null);
  }
  assert.equal(recordingSaveProgress({ ...snapshot, phase: "idle" }), null);
});

const importing: ImportSnapshot = { revision: 2, token: "audio", phase: "copying", busy: true, recordingBlocked: false, fileName: "会議.wav", completed: 50, sourceBytes: 200, requiredBytes: 1000, availableBytes: 2000, info: { schemaVersion: 1, detectedFormat: "wav", codec: "pcm", sampleRate: 16000, channels: 1, frames: 100, durationUs: 6250 }, saved: null, errorMessage: null };
const job: NotificationJob = { id: "job-a", meetingId: "meeting-a", meetingTitle: "会議A", kind: "transcription", state: "running", completedUnits: 1, totalUnits: 4, errorCode: null, outputAnalysisId: null, pauseReason: null, specHash: "a".repeat(64), inputHash: "b".repeat(64), operationId: "operation", retryOfJobId: null, attempt: 1, outputMinutesId: null, createdAt: 10, updatedAt: 20 };

test("percent handles unknown or invalid totals and never rounds unfinished work to 100", () => {
  assert.equal(progressPercent(0, 200), 0);
  assert.equal(progressPercent(50, 200), 25);
  assert.equal(progressPercent(999, 1000), 99);
  assert.equal(progressPercent(200, 200), 100);
  for (const [done, total] of [[null, 100], [0, null], [0, 0], [-1, 100], [101, 100], [NaN, 100], [0, Infinity], [1.5, 100]]) assert.equal(progressPercent(done, total), null);
});

test("import changes units by phase and hides terminal operations", () => {
  assert.equal(progressPercent(importProgress(importing)!.completed, importProgress(importing)!.total), 25);
  const conversion = importProgress({ ...importing, phase: "converting" })!;
  assert.equal(progressPercent(conversion.completed, conversion.total), 50);
  assert.match(conversion.detail!, /フレーム/);
  for (const phase of ["choosing", "inspecting", "waiting", "decoding", "saving", "recovering", "cancelling"]) assert.equal(importProgress({ ...importing, phase })!.total, null);
  for (const phase of ["saved", "cancelled", "failed", "ready"]) assert.equal(importProgress({ ...importing, phase, busy: false }), null);
  assert.equal(importProgress(null), null);
  assert.equal(importProgress({ ...importing, completed: 201 })!.total, null);
});

test("old import replies cannot resurrect a completed progress bar", () => {
  const saved = { ...importing, revision: 3, phase: "saved", busy: false };
  assert.equal(importProgress(acceptImport(saved, importing)), null);
});

test("transfer updates are bound to operation, recipe and transfer size, and never regress", () => {
  const event = { operationId: "operation-a", recipeHash: "hash", receivedBytes: 50, totalBytes: 200 };
  const accept = (value: unknown) => acceptTransferProgress(null, value, "operation-a", "hash", 200);
  assert.deepEqual(accept(event), { receivedBytes: 50, totalBytes: 200 });
  for (const patch of [{ operationId: "old" }, { recipeHash: "other" }, { totalBytes: 100 }, { receivedBytes: 201 }, { receivedBytes: -1 }, { receivedBytes: NaN }]) assert.equal(accept({ ...event, ...patch }), null);
  const current = accept(event)!;
  assert.equal(acceptTransferProgress(current, { ...event, receivedBytes: 20 }, "operation-a", "hash", 200), current);
  assert.equal(accept(null), null);
});

test("download completion switches to verification instead of claiming model readiness", () => {
  const downloading = modelProgress("モデルA", "download", { receivedBytes: 50, totalBytes: 200 });
  assert.equal(progressPercent(downloading.completed, downloading.total), 25);
  const verifying = modelProgress("モデルA", "download", { receivedBytes: 200, totalBytes: 200 });
  assert.equal(verifying.total, null);
  assert.match(verifying.label, /検証中/);
  for (const stage of ["place", "check", "preview", "apply"]) assert.equal(modelProgress("モデルA", stage, { receivedBytes: 200, totalBytes: 200 }).total, null);
});

test("transcription counts durable units and preserves paused or interrupted progress", () => {
  const running = jobProgress(job, job.meetingTitle)!;
  assert.equal(progressPercent(running.completed, running.total), 25);
  assert.match(running.detail!, /1 \/ 4 区間完了/);
  assert.match(running.detail!, /約2分/);
  for (const state of ["paused", "interrupted"]) {
    const paused = jobProgress({ ...job, state }, job.meetingTitle)!;
    assert.equal(paused.paused, true);
    assert.equal(paused.completed, 1);
  }
  assert.equal(jobProgress({ ...job, totalUnits: null }, "会議A")!.total, null);
  assert.equal(jobProgress({ ...job, completedUnits: 0, totalUnits: 1 }, "短い会議")!.completed, 0);
  assert.equal(jobProgress({ ...job, completedUnits: 4 }, "会議A")!.total, null);
  assert.match(jobProgress({ ...job, completedUnits: 4 }, "会議A")!.label, /保存中/);
  for (const state of ["succeeded", "failed", "cancelled"]) assert.equal(jobProgress({ ...job, state }, "会議A"), null);
  const minutes = jobProgress({ ...job, kind: "minutes_generation" }, "会議A", "meeting-a")!;
  assert.equal(minutes.total, 4);
  assert.equal(minutes.kind, "minutes_generation");
  assert.equal(minutes.meetingId, "meeting-a");
  assert.match(minutes.label, /議事録生成/);
});

test("model waiting and unresolved steps remain visible without a stale download percentage", () => {
  const transfer = { receivedBytes: 200, totalBytes: 200 };
  const waiting = modelProgress("Qwen 3 8B", "place", transfer, "queued");
  assert.equal(waiting.state, "queued");
  assert.match(waiting.label, /AI処理の完了待ち/);
  assert.equal(waiting.total, null);
  const interrupted = modelProgress("Qwen 3 8B", "place", transfer, "interrupted");
  assert.equal(interrupted.state, "interrupted");
  assert.equal(interrupted.completed, null);
  assert.match(interrupted.detail!, /自動で確認/);
  assert.equal(interrupted.paused, true);
  const recovering = modelProgress("Qwen 3 8B", "place", transfer, "recovering");
  assert.equal(recovering.state, "running");
  assert.equal(recovering.completed, null);
  assert.equal(recovering.total, null);
  assert.equal(recovering.paused, false);
  assert.match(recovering.label, /再試行を準備中/);
});

test("diarization waits for measured native progress", () => {
  const progress = jobProgress({ ...job, kind: "diarization", totalUnits: 2 }, "会議A")!;
  assert.equal(progressPercent(progress.completed, progress.total), null);
  assert.match(progress.label, /話者分離/);
  assert.doesNotMatch(progress.detail!, /約2分/);
});

test("global job progress uses reconciled snapshots after stale reads and retries", () => {
  const tracker = new JobNotificationTracker(0);
  tracker.ingest([job]);
  const old = tracker.ingest([{ ...job, completedUnits: 0, updatedAt: 15 }]);
  assert.equal(old.jobs[0].completedUnits, 1);
  tracker.ingest([{ ...job, state: "cancelled", updatedAt: 30 }]);
  const stale = tracker.ingest([job]);
  assert.equal(jobProgress(stale.jobs[0], "会議A"), null);
  const retried = { ...job, id: "retry", retryOfJobId: job.id, operationId: "retry-operation", attempt: 2, createdAt: 31, updatedAt: 31 };
  const next = tracker.ingest([retried, { ...job, state: "cancelled", updatedAt: 30 }]);
  assert.deepEqual(next.jobs.map(row => jobProgress(row, row.meetingTitle)).filter(Boolean).map(row => row!.id), ["job:retry"]);
});

test("progress owners coexist, publish only changes, and clear independently", () => {
  const registry = new ProgressRegistry();
  let changes = 0;
  const unsubscribe = registry.subscribe(() => changes++);
  registry.set("import", [importProgress(importing)!]);
  registry.set("jobs", [jobProgress(job, "会議A")!, jobProgress({ ...job, id: "job-b" }, "会議B")!]);
  const snapshot = registry.getSnapshot();
  registry.set("import", [importProgress(importing)!]);
  assert.equal(registry.getSnapshot(), snapshot);
  assert.equal(changes, 2);
  assert.equal(snapshot.length, 3);
  registry.set("import", []);
  assert.equal(registry.getSnapshot().length, 2);
  unsubscribe();
  registry.set("jobs", []);
  assert.equal(changes, 3);
  assert.deepEqual(registry.getSnapshot(), []);
});

test("job inventory includes waiting work, prioritizes running work without mutating the source", () => {
  const waiting = jobProgress({ ...job, state: "queued" }, "会議A")!;
  const paused = jobProgress({ ...job, id: "paused", state: "paused" }, "会議B")!;
  const running = modelProgress("モデルA", "download", { receivedBytes: 1, totalBytes: 2 });
  const items = [paused, waiting, running];
  assert.deepEqual(orderedProgress(items), [running, waiting, paused]);
  assert.deepEqual(items, [paused, waiting, running]);
  assert.equal(importProgress({ ...importing, phase: "waiting" })!.state, "queued");
  assert.equal(importProgress({ ...importing, phase: "cancelling" })!.state, "pausing");
});

test("read status distinguishes loading, empty success and a failed read with retained jobs", () => {
  const registry = new ProgressRegistry();
  assert.equal(registry.getReadSnapshot().state, "loading");
  registry.setReadStatus("");
  assert.equal(registry.getReadSnapshot().state, "ready");
  const ready = registry.getReadSnapshot();
  registry.setReadStatus("");
  assert.equal(registry.getReadSnapshot(), ready);
  registry.set("jobs", [jobProgress(job, "会議A")!]);
  registry.setReadStatus("read failed");
  assert.equal(registry.getReadSnapshot().state, "error");
  assert.equal(registry.getSnapshot().length, 1);
  registry.setReadStatus(null);
  assert.equal(registry.getReadSnapshot().state, "loading");
});


test("minutes reports analysis counts and percentages through pause, saving and completion", () => {
  const minute = { ...job, kind: "minutes_generation" };
  for (const [done, total, percent] of [[0, 15, 0], [2, 10, 20], [7, 10, 70], [14, 15, 93]]) {
    const progress = jobProgress({ ...minute, completedUnits: done, totalUnits: total }, "会議A")!;
    assert.equal(progress.label, `議事録生成 · 解析中 ${done} / ${total}`);
    assert.equal(progressPercent(progress.completed, progress.total), percent);
  }
  for (const [state, status] of [["queued", "待機中"], ["paused", "一時停止"], ["interrupted", "中断"], ["pausing", "停止処理中"]]) {
    const progress = jobProgress({ ...minute, state, completedUnits: 2, totalUnits: 10 }, "会議A")!;
    assert.equal(progress.label, `議事録生成 · ${status} 2 / 10`);
    assert.equal(progressPercent(progress.completed, progress.total), 20);
  }
  const saving = jobProgress({ ...minute, completedUnits: 10, totalUnits: 10 }, "会議A")!;
  assert.equal(saving.label, "議事録生成 · 結果を保存中 10 / 10");
  assert.equal(progressPercent(saving.completed, saving.total), 100);
  for (const totalUnits of [null, 0, 1]) {
    const unknown = jobProgress({ ...minute, completedUnits: 2, totalUnits }, "会議A")!;
    assert.equal(unknown.label, "議事録生成 · 解析中");
    assert.equal(progressPercent(unknown.completed, unknown.total), null);
  }
  for (const state of ["succeeded", "failed", "cancelled"]) assert.equal(jobProgress({ ...minute, state }, "会議A"), null);
});


test("whole-job elapsed time survives unit changes, saving, pause and authoritative reloads", () => {
  const start = 10000, now = start + 320999;
  for (const kind of ["minutes_generation", "transcription"]) {
    for (const state of ["running", "paused", "queued", "interrupted"]) {
      for (const completedUnits of [0, 2, 4]) {
        const progress = jobProgress({ ...job, kind, state, createdAt: start, completedUnits }, "会議A")!;
        assert.equal(operationElapsed(progress, now), "全体の経過時間 5分20秒");
        const reloaded = jobProgress({ ...job, kind, state, createdAt: start, completedUnits }, "会議A")!;
        assert.equal(operationElapsed(reloaded, now + 1000), "全体の経過時間 5分21秒");
      }
    }
  }
  const fresh = jobProgress({ ...job, id: "new-job", createdAt: now }, "会議A")!;
  assert.equal(operationElapsed(fresh, now), "全体の経過時間 0分0秒");
});

test("model elapsed time uses the same start through download, verification, placement and application", () => {
  for (const stage of ["download", "place", "check", "preview", "apply"]) {
    for (const state of ["running", "queued", "interrupted"] as const) {
      const progress = modelProgress("モデルA", stage, { receivedBytes: 200, totalBytes: 200 }, state, 10000);
      assert.equal(operationElapsed(progress, 330000), "全体の経過時間 5分20秒");
    }
  }
});

test("elapsed time handles missing or invalid starts and clocks behind the start", () => {
  const progress = jobProgress(job, "会議A")!;
  for (const startedAtMs of [undefined, NaN, Infinity, -1, 1.5]) assert.equal(operationElapsed({ ...progress, startedAtMs }, 1000), undefined);
  assert.equal(operationElapsed({ ...progress, startedAtMs: 1000 }, 999), "全体の経過時間 0分0秒");
  assert.equal(operationElapsed({ ...progress, startedAtMs: 0 }, 3600000), "全体の経過時間 60分0秒");
});

test("speaker completion shows transient work separately from durable result units", () => {
  const completion = { ...job, kind: "speaker_completion", completedUnits: 0, totalUnits: 1,
    executionProgress: { phase: "matching" as const, completed: 14, total: 33, phaseCompleted: 12, phaseTotal: 30 } };
  const p = jobProgress(completion, "会議A")!;
  assert.equal(progressPercent(p.completed, p.total), 40);
  assert.match(p.detail!, /発言照合 12／30件/);
  assert.match(p.label, /発言照合中/);
  const saving = jobProgress({ ...completion, executionProgress: { ...completion.executionProgress, phase: "saving", completed: 32, phaseCompleted: 0, phaseTotal: 1 } }, "会議A")!;
  assert.equal(progressPercent(saving.completed, saving.total), null);
  assert.match(saving.label, /結果を保存中/);
  for (const state of ["paused", "interrupted", "failed", "cancelled", "queued"]) {
    const view = jobProgress({ ...completion, state }, "会議A");
    assert.equal(view?.total ?? null, null);
    assert.doesNotMatch(view?.label ?? "", /発言照合中/);
  }
  assert.equal(jobProgress({ ...completion, state: "succeeded", completedUnits: 1 }, "会議A"), null);
  assert.equal(jobProgress({ ...completion, executionProgress: undefined }, "会議A")!.total, null);
  for (const patch of [{ completed: 33 }, { total: 0 }, { completed: -1 }, { phaseCompleted: 31 }, { completed: 1.5 }, { total: NaN }]) {
    assert.equal(jobProgress({ ...completion, executionProgress: { ...completion.executionProgress, ...patch } }, "会議A")!.total, null);
  }
  const large = jobProgress({ ...completion, executionProgress: { phase: "saving", completed: 999, total: 1000, phaseCompleted: 0, phaseTotal: 1 } }, "会議A")!;
  assert.equal(progressPercent(large.completed, large.total), null);
});
