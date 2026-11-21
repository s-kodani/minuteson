import { recoveryMeetingState, type RecoverySnapshot } from "./recording-recovery-state.ts";
import type { BridgeState } from "./session";
import { meetingDiagnosticSummary, type MeetingDiagnostic } from "./meeting-diagnostics.ts";

interface MeetingStatusSource {
  id: string;
  lifecycle: string;
  sourceType?: "recording" | "audio_import" | null;
}

// Presentation only. Never repair persisted sessions, grant permission to
// restart/delete, or treat a stored lifecycle as proof of a running helper.
export function meetingStatusLabel(meeting: MeetingStatusSource, state: BridgeState, recovery?: RecoverySnapshot | null, diagnostics?: readonly MeetingDiagnostic[]): string {
  const diagnostic = meetingDiagnosticSummary(diagnostics);
  if (diagnostic && diagnostic.reason !== "audio_recovery") return diagnostic.label;
  const item = recoveryMeetingState(recovery ?? null, meeting.id);
  if (item) return item.phase === "running" ? "録音を保存中" : item.phase === "queued" ? "録音の保存待ち" : item.noAudio ? "録音失敗" : "音声保存エラー";
  if (diagnostic) return diagnostic.label;
  if (recovery?.items.some(i => i.meetingId === meeting.id && i.phase === "succeeded")) return "録音済み";
  if (state.kind === "ready" && state.snapshot.meetingId === meeting.id && state.snapshot.phase === "saving") return "音声保存中";
  if (meeting.lifecycle === "imported") return "取り込み済み";
  if (meeting.lifecycle === "recorded") return "録音済み";
  if (meeting.lifecycle !== "recording" && !(meeting.lifecycle === "draft" && meeting.sourceType === "recording")) return "下書き";
  if (state.kind !== "ready") return "録音状態を確認中";
  if (state.snapshot.meetingId !== meeting.id) return "音声保存エラー";
  return {
    idle: "音声保存エラー",
    preparing: "録音準備中",
    recording: "録音中",
    stopping: "録音停止中",
    saving: "音声保存中",
    error: "音声保存エラー",
  }[state.snapshot.phase];
}
