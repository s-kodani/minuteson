import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { meetingStatusLabel } from "../src/meeting-status.ts";
import type { BridgeState, RecordingPhase } from "../src/session.ts";

const meeting = { id: "incident", lifecycle: "recording", sourceType: "recording" as const };
const ready = (phase: RecordingPhase, meetingId: string | null = meeting.id): BridgeState => ({
  kind: "ready", snapshot: { phase, meetingId, revision: 1, elapsedMs: 0,
    microphoneConnected: false, systemAudioConnected: false, sessionId: "session", errorMessage: null },
});

test("stored recording without its live owner is not shown as running", () => {
  for (const state of [ready("idle", null), ready("recording", "other"), ready("error", "other")]) {
    assert.equal(meetingStatusLabel(meeting, state), "録音未完了・要確認");
  }
  assert.equal(meetingStatusLabel(meeting, ready("idle")), "録音未完了・要確認");
});

test("unavailable runtime is unknown, not fabricated interruption or recording", () => {
  for (const state of [{kind:"loading"}, {kind:"unavailable"}, {kind:"error", message:"offline"}] as BridgeState[]) {
    assert.equal(meetingStatusLabel(meeting, state), "録音状態を確認中");
  }
});

test("matching owner presents each actual recording phase", () => {
  for (const [phase, label] of Object.entries({ preparing:"録音準備中", recording:"録音中",
    stopping:"録音停止中", saving:"音声保存中", error:"録音エラー・要確認" })) {
    assert.equal(meetingStatusLabel(meeting, ready(phase as RecordingPhase)), label);
  }
});

test("failed recording drafts remain distinct from untouched drafts and saved audio", () => {
  assert.equal(meetingStatusLabel({...meeting,lifecycle:"draft"},ready("idle",null)),"録音未完了・要確認");
  assert.equal(meetingStatusLabel({...meeting,lifecycle:"draft",sourceType:null},ready("idle",null)),"下書き");
  assert.equal(meetingStatusLabel({...meeting,lifecycle:"recorded"},{kind:"loading"}),"録音済み");
  assert.equal(meetingStatusLabel({...meeting,lifecycle:"imported"},{kind:"loading"}),"取り込み済み");
});

test("recording labels stay independent of live deletion admission and restart eligibility", () => {
  const source=readFileSync(new URL("../src/main.tsx",import.meta.url),"utf8");
  assert.ok(source.includes("meetingStatusLabel(meeting, state, recovery.snapshot)"));
  const menu=readFileSync(new URL("../src/meeting-deletion-menu.tsx",import.meta.url),"utf8");
  assert.ok(source.includes("<MeetingDeletionMenu recoveryRunning="));
  assert.ok(menu.includes('!deletion.canReviewMeetingId(meeting.id)'));
  assert.ok(!menu.includes('meeting.lifecycle === "recording"'));
  assert.ok(source.includes('selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null'));
});
