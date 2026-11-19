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
test("empty and repeated pages retain the collection and unchanged entry objects", () => {
  const current = [entry(1), entry(2)];
  for (const direction of ["older", "newer"] as const) {
    assert.equal(mergeLogEntries(current, [], direction).entries, current);
    assert.equal(mergeLogEntries(current, structuredClone(current), direction).entries, current);
  }
  assert.equal(mergeLogEntries(current, structuredClone(current), "initial").entries, current);
  const replaced = mergeLogEntries(current, [structuredClone(current[1])], "initial").entries;
  assert.deepEqual(replaced.map(row => row.id), [2]);
  assert.equal(replaced[0], current[1]);
  assert.deepEqual(mergeLogEntries(current, [], "initial").entries, []);
});
test("changed log fields replace their row while retaining every unchanged sibling", () => {
  const current = [entry(1), entry(2)];
  const changes: Partial<InternalLogEntry>[] = [
    { timestamp: 1792159999999 }, { sessionId: "another-session" }, { kind: "engine" }, { level: "warn" },
    { category: "playback" }, { meetingId: null }, { jobId: null }, { operationId: "operation-1" },
    { state: "succeeded" }, { elapsedMs: null }, { message: "changed text" }, { stream: "stderr" }, { source: "/another/source" },
  ];
  for (const patch of changes) {
    const changed = entry(2, patch);
    const merged = mergeLogEntries(current, [changed], "newer").entries;
    assert.notEqual(merged, current);
    assert.equal(merged[0], current[0]);
    assert.equal(merged[1], changed);
  }
});
test("a page outside the retained cap signals trimming without replacing visible rows", () => {
  const current = Array.from({ length: 1000 }, (_, index) => entry(index + 1));
  const result = mergeLogEntries(current, [entry(0)], "newer");
  assert.equal(result.entries, current);
  assert.equal(result.trimmed, true);
});
test("copy keeps raw multiline content, identifiers and stream without interpreting markup", () => {
  const raw = "<script>window.bad = true</script>\n会議本文🦀\n";
  const text = copyLogText([entry(1, { kind: "engine", level: "raw", elapsedMs: null, stream: "stderr.log", message: raw })]);
  assert.ok(text.endsWith(raw)); assert.match(text, /meeting=meeting-1 job=job-1 session=session/); assert.match(text, /stderr\.log/); assert.doesNotMatch(text, /elapsed=null/);
});
test("elapsed time distinguishes missing values and durations", () => {
  assert.equal(logElapsed(null), "—"); assert.equal(logElapsed(0), "0 ms"); assert.equal(logElapsed(1500), "1.5 秒"); assert.equal(logElapsed(61500), "1 分 1 秒");
});

test("copy formats paths in message and source without mutating retrieved logs or identifiers", () => {
  const source=String.raw`\\?\C:\実行 単位\worker.exe`,message=`開けません: ${JSON.stringify(String.raw`\\?\UNC\server\共有\会議`)}\n${String.raw`\\?\C:\会議\audio.wav`}`;
  const original=Object.freeze(entry(1,{source,message,operationId:"operation-original"}));
  const before=JSON.stringify(original),copied=copyLogText([original]);
  assert.ok(copied.includes(String.raw`source=C:\実行 単位\worker.exe`));
  assert.ok(copied.includes(JSON.stringify(String.raw`\\server\共有\会議`)));
  assert.ok(copied.includes(String.raw`C:\会議\audio.wav`));
  assert.match(copied,/operation=operation-original/);
  assert.equal(JSON.stringify(original),before);
});
