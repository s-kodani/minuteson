import { recoveryMeetingState, type RecoverySnapshot } from "./recording-recovery-state.ts";
import type { BridgeState } from "./session";

interface MeetingStatusSource {
  id: string;
  lifecycle: string;
  sourceType?: "recording" | "audio_import" | null;
}

// Presentation only. Never repair persisted sessions, grant permission to
// restart/delete, or treat a stored lifecycle as proof of a running helper.
export function meetingStatusLabel(meeting: MeetingStatusSource, state: BridgeState, recovery?: RecoverySnapshot | null): string {
  const item = recoveryMeetingState(recovery ?? null, meeting.id);
  if (item && (item.phase !== "failed" || !["recorded", "imported"].includes(meeting.lifecycle))) return item.phase === "running" ? "録音を回復中" : item.phase === "queued" ? "録音の回復待ち" : "録音未完了・要確認";
  if (meeting.lifecycle === "imported") return "取り込み済み";
  if (meeting.lifecycle === "recorded") return "録音済み";
  if (meeting.lifecycle !== "recording" && !(meeting.lifecycle === "draft" && meeting.sourceType === "recording")) return "下書き";
  if (state.kind !== "ready") return "録音状態を確認中";
  if (state.snapshot.meetingId !== meeting.id) return "録音未完了・要確認";
  return {
    idle: "録音未完了・要確認",
    preparing: "録音準備中",
    recording: "録音中",
    stopping: "録音停止中",
    saving: "音声保存中",
    error: "録音エラー・要確認",
  }[state.snapshot.phase];
}
