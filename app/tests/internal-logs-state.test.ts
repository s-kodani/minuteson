import assert from "node:assert/strict";
import test from "node:test";
import { copyLogText, logElapsed, mergeLogEntries, type InternalLogEntry } from "../src/internal-logs-state.ts";
const entry = (id: number, patch: Partial<InternalLogEntry> = {}): InternalLogEntry => ({ id, timestamp: 1792150000000 + id, sessionId: "session", kind: "process", level: "info", category: "transcription", meetingId: "meeting-1", jobId: "job-1", operationId: null, state: "running", elapsedMs: 1200, message: `log ${id}`, stream: null, ...patch });
test("log history and delta pages merge in stable chronological order without duplicates", () => {
  assert.deepEqual(mergeLogEntries([entry(3), entry(4)], [entry(1), entry(2), entry(3)], "older").entries.map(e => e.id), [1, 2, 3, 4]);
  assert.deepEqual(mergeLogEntries([entry(1), entry(2)], [entry(2), entry(4), entry(3)], "newer").entries.map(e => e.id), [1, 2, 3, 4]);
});
test("viewer keeps at most 1000 entries in the requested direction and signals removed rows", () => {
  const entries = Array.from({ length: 1000 }, (_, index) => entry(index + 1));
  const delta = mergeLogEntries(entries, [entry(1001)], "newer");
  assert.equal(delta.entries.length, 1000); assert.equal(delta.entries[0].id, 2); assert.equal(delta.trimmed, true);
  const older = mergeLogEntries(entries, [entry(0)], "older");
  assert.equal(older.entries[0].id, 0); assert.equal(older.entries.at(-1)!.id, 999); assert.equal(older.trimmed, true);
});
test("copy keeps raw multiline content, identifiers and stream without interpreting markup", () => {
  const raw = "<script>window.bad = true</script>\n会議本文🦀\n";
  const text = copyLogText([entry(1, { kind: "engine", level: "raw", elapsedMs: null, stream: "stderr.log", message: raw })]);
  assert.ok(text.endsWith(raw)); assert.match(text, /meeting=meeting-1 job=job-1 session=session/); assert.match(text, /stderr\.log/); assert.doesNotMatch(text, /elapsed=null/);
});
test("elapsed time distinguishes missing values and durations", () => {
  assert.equal(logElapsed(null), "—"); assert.equal(logElapsed(0), "0 ms"); assert.equal(logElapsed(1500), "1.5 秒"); assert.equal(logElapsed(61500), "1 分 1 秒");
});
