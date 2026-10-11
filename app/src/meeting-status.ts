import type { BridgeState } from "./session";

interface MeetingStatusSource {
  id: string;
  lifecycle: string;
  sourceType?: "recording" | "audio_import" | null;
}

// Presentation only. Never repair persisted sessions, grant permission to
// restart/delete, or treat a stored lifecycle as proof of a running helper.
export function meetingStatusLabel(meeting: MeetingStatusSource, state: BridgeState): string {
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
