import type { BridgeState } from "./session";

export function phaseLabel(state: BridgeState): string {
  if (state.kind === "loading") return "接続中";
  if (state.kind === "unavailable") return "デスクトップ接続なし";
  if (state.kind === "error") return "接続エラー";
  return {
    idle: "待機中",
    preparing: "準備中",
    recording: "録音中",
    stopping: "停止中",
    saving: "保存中",
    error: "要対応",
  }[state.snapshot.phase];
}
