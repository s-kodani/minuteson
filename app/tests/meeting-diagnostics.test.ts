import assert from "node:assert/strict";
import test from "node:test";
import { meetingDiagnosticPresentation, meetingDiagnosticSummary } from "../src/meeting-diagnostics.ts";
import { meetingStatusLabel } from "../src/meeting-status.ts";
import type { RecoverySnapshot } from "../src/recording-recovery-state.ts";

const issue = (reason: string) => ({ reason, message: "診断の詳細" });
const meeting = { id: "meeting", lifecycle: "recorded" };

test("missing and unreadable audio have distinct labels and retain document use", () => {
  for (const [reason, label] of [["audio_missing", "音声なし"], ["audio_unreadable", "音声を利用できません"], ["audio_unavailable", "音声を利用できません"]]) {
    const presentation = meetingDiagnosticPresentation(issue(reason));
    assert.equal(presentation.label, label);
    assert.match(presentation.description, /音声の再生・再解析はできません/);
    assert.match(presentation.description, /文字起こし・議事録は引き続き閲覧・編集できます/);
    assert.equal(meetingStatusLabel(meeting, { kind: "loading" }, null, [issue(reason)]), label);
  }
});

test("unreadable meeting data explains its protection and the available deletion action", () => {
  const presentation = meetingDiagnosticPresentation(issue("meeting_row_invalid"));
  assert.equal(presentation.label, "会議データを利用できません");
  assert.match(presentation.description, /編集・解析を停止/);
  assert.match(presentation.description, /修復する機能はありません/);
  assert.match(presentation.description, /会議の操作メニューから「削除」/);
  assert.doesNotMatch(presentation.description, /閲覧できません/);
});

test("audio notices do not promise editing when meeting or workspace protection blocks it", () => {
  for (const reason of ["audio_missing", "audio_unreadable", "audio_unavailable"]) {
    const presentation = meetingDiagnosticPresentation(issue(reason), { documentEditingBlocked: true });
    assert.match(presentation.description, /保存済みの文字起こし・議事録は保持しています/);
    assert.doesNotMatch(presentation.description, /閲覧・編集できます/);
  }
});

test("pending deletion never offers an unavailable continuation or repair", () => {
  for (const reason of ["deletion_pending", "audio_deletion_pending"]) {
    const presentation = meetingDiagnosticPresentation(issue(reason));
    assert.match(presentation.label, /削除が未完了/);
    assert.match(presentation.description, /この画面から.*できません/);
    assert.doesNotMatch(presentation.description, /メニュー|再起動|続行|再試行/);
  }
});

test("meeting protection wins over audio-only notices independently of diagnostic order", () => {
  const items = [issue("audio_missing"), issue("meeting_row_invalid"), issue("salvage_minutes")];
  for (const ordered of [items, [...items].reverse()]) {
    assert.equal(meetingDiagnosticSummary(ordered)?.label, "会議データを利用できません");
  }
  assert.equal(meetingDiagnosticSummary(undefined), null);
  assert.equal(meetingDiagnosticSummary([]), null);
});

test("partial salvage describes the past result without forbidding new document use", () => {
  for (const reason of ["salvage_meeting", "salvage_transcript", "salvage_draft", "salvage_minutes", "salvage_vocabulary"]) {
    const presentation = meetingDiagnosticPresentation(issue(reason));
    assert.match(presentation.label, /復元できませんでした/);
    assert.match(presentation.description, /復元できたデータは引き続き利用できます/);
    assert.doesNotMatch(presentation.description, /削除|停止|再試行/);
  }
});

test("recording recovery retains its live save phase instead of a generic diagnostic label", () => {
  const snapshot: RecoverySnapshot = { runId: "run", revision: 1, phase: "running", origin: "startup", targetSessionId: null, error: null,
    items: [{ sessionId: "session", meetingId: meeting.id, meetingTitle: "会議", phase: "running", startedAtMs: 1, error: null }] };
  assert.equal(meetingStatusLabel(meeting, { kind: "loading" }, snapshot, [issue("audio_recovery")]), "録音を保存中");
  assert.equal(meetingStatusLabel(meeting, { kind: "loading" }, null, [issue("audio_recovery")]), "音声保存エラー");
});

test("unrecognized diagnostic retains its reason text without invented recovery advice", () => {
  const presentation = meetingDiagnosticPresentation(issue("future_reason"));
  assert.equal(presentation.description, "診断の詳細");
  assert.doesNotMatch(presentation.label, /要確認|要対応/);
});
