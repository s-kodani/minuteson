import test from "node:test";
import assert from "node:assert/strict";
import {
  checkedPartialRecordingSummary, canRecoverPartialRecording, partialLaneLabel,
  partialLaneStateLabel, partialTimelineLabel,
} from "../src/partial-recording-state.ts";
const meeting = "735027d4-a03e-43d3-830a-21e767dfaf88";
const session = "735027d4-a03e-43d3-830a-21e767dfaf89";
const available = () => ({ lane: "microphone", state: "available", sealedSegments: 2, frames: 34000,
  startUs: 0, endUs: 4_950_000, sampleRate: 8000, channels: 2 });
const absent = (state = "missing") => ({ lane: "system", state, sealedSegments: 0, frames: 0,
  startUs: null, endUs: null, sampleRate: null, channels: null });
const report = () => ({ meetingId: meeting, sessionId: session, reviewHash: "a".repeat(64), durationUs: 4_950_000,
  lanes: [available(), absent()], acquisitionEndKnown: false, tailLossUs: null });
const check = (value: unknown) => checkedPartialRecordingSummary(value, meeting, session);
test("native sealed-prefix summary keeps unknown closure and tail and copies the input", () => {
  const input = report(), result = check(input);
  assert.equal(canRecoverPartialRecording(result), true);
  assert.equal(result.acquisitionEndKnown, false); assert.equal(result.tailLossUs, null);
  input.lanes[0].state = "unreadable"; assert.equal(result.lanes[0].state, "available");
});
test("empty, missing and unreadable inputs remain distinct without fabricated measurements", () => {
  for (const state of ["empty", "missing", "unreadable"]) {
    const result = check({ ...report(), lanes: [available(), absent(state)] });
    assert.equal(result.lanes[1].state, state); assert.equal(result.lanes[1].frames, 0);
    assert.notEqual(partialLaneStateLabel(result.lanes[1].state), partialLaneStateLabel("available"));
  }
});
test("no available prefix is a valid review but cannot be recovered", () => {
  const result = check({ ...report(), durationUs: 0, lanes: [{ ...absent("empty"), lane: "microphone" }, absent()] });
  assert.equal(canRecoverPartialRecording(result), false);
});
test("another owner, unknown fields, noncanonical IDs and malformed generations are rejected", () => {
  for (const value of [null, [], { ...report(), sessionId: meeting }, { ...report(), meetingId: session },
    { ...report(), meetingId: meeting.toUpperCase() }, { ...report(), reviewHash: "A".repeat(64) },
    { ...report(), reviewHash: "a".repeat(63) }, { ...report(), schemaVersion: 2 }]) assert.throws(() => check(value));
});
test("ordered unique lanes and exact measured versus unmeasured fields are required", () => {
  for (const lanes of [[], [available()], [absent(), available()], [available(), available()],
    [available(), { ...absent(), frames: 1 }], [available(), { ...absent(), startUs: 0 }],
    [available(), { ...absent(), state: "saved" }], [{ ...available(), extra: true }, absent()]])
    assert.throws(() => check({ ...report(), lanes }));
});
test("out of range or nonfinite measurements never become a recoverable preview", () => {
  for (const lane of [{ ...available(), frames: 0 }, { ...available(), frames: Number.MAX_SAFE_INTEGER + 1 },
    { ...available(), sealedSegments: 32769 }, { ...available(), sealedSegments: 1.5 },
    { ...available(), startUs: -1 }, { ...available(), endUs: 0 }, { ...available(), endUs: Infinity },
    { ...available(), sampleRate: 7999 }, { ...available(), sampleRate: 8000.5 },
    { ...available(), channels: 0 }, { ...available(), channels: 9 }])
    assert.throws(() => check({ ...report(), lanes: [lane, absent()] }));
});
test("extent must equal the latest saved endpoint and origin must be the earliest known sample", () => {
  for (const durationUs of [0, 4_949_999, 4_950_001, NaN]) assert.throws(() => check({ ...report(), durationUs }));
  assert.throws(() => check({ ...report(), lanes: [{ ...available(), startUs: 50 }, absent()] }));
  const result = check({ ...report(), durationUs: 4_950_050, lanes: [available(),
    { ...available(), lane: "system", startUs: 50, endUs: 4_950_050 }] });
  assert.equal(result.lanes[1].startUs, 50); assert.equal(result.durationUs, 4_950_050);
});
test("closure or measured loss claims cannot be inserted into partial recovery data", () => {
  for (const value of [{ ...report(), acquisitionEndKnown: true }, { ...report(), tailLossUs: 0 },
    { ...report(), tailLossUs: 5_000_000 }, { ...report(), acquisitionEndKnown: undefined }])
    assert.throws(() => check(value));
});
test("labels preserve input identity and exact microseconds without claiming an acquisition end", () => {
  assert.equal(partialLaneLabel("microphone"), "マイク"); assert.equal(partialLaneLabel("system"), "PC音声");
  assert.equal(partialTimelineLabel(50), "0.000050秒"); assert.equal(partialTimelineLabel(4_950_050), "4.950050秒");
  for (const value of [-1, 0.5, Infinity]) assert.throws(() => partialTimelineLabel(value));
});
