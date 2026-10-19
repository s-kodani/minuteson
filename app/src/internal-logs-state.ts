export interface InternalLogEntry {
  id: number;
  timestamp: number;
  sessionId: string;
  kind: "process" | "engine";
  level: string;
  category: string;
  meetingId: string | null;
  jobId: string | null;
  operationId: string | null;
  state: string;
  elapsedMs: number | null;
  message: string;
  stream: string | null;
  source?: string | null;
}
export interface InternalLogFilter {
  kind: "process" | "engine";
  level?: string;
  category?: string;
  relatedId?: string;
  text?: string;
  from?: number;
  to?: number;
}
export interface InternalLogPage {
  entries: InternalLogEntry[];
  olderCursor: number;
  newerCursor: number;
  hasMore: boolean;
  expired: boolean;
  status: { storageError: string | null; droppedEntries: number; historyTruncated: boolean };
}
export const logCategories: Record<string, string> = {
  application: "アプリ", recording: "録音", import: "音声取り込み", model: "モデル準備",
  speaker_completion: "話者補完", transcription: "文字起こし", diarization: "話者分離", minutes_generation: "議事録生成",
  playback: "再生準備", backup: "バックアップ", restore: "復元", relocation: "保存先変更",
  deletion: "会議削除", audiodeletion: "音声削除", data: "データ管理", analysis: "解析", recovery: "回復処理",
};
export const logStates: Record<string, string> = {
  starting: "開始", ready: "準備完了", idle: "待機", queued: "待機", waiting: "完了待ち", preparing: "準備中",
  recording: "録音中", stopping: "停止中", saving: "保存中", running: "処理中", succeeded: "完了",
  failed: "失敗", error: "エラー", pausing: "一時停止中", paused: "一時停止", cancelled: "取消済み",
  interrupted: "中断", stopped: "終了", verifying: "照合中", input: "入力処理中", generating: "生成中", output: "出力",
};
export function mergeLogEntries(current: InternalLogEntry[], incoming: InternalLogEntry[], direction: "older" | "newer") {
  const unique = new Map(current.map(entry => [entry.id, entry]));
  incoming.forEach(entry => unique.set(entry.id, entry));
  const entries = [...unique.values()].sort((a, b) => a.id - b.id);
  return { entries: direction === "older" ? entries.slice(0, 1000) : entries.slice(-1000), trimmed: entries.length > 1000 };
}
export function logElapsed(ms: number | null) {
  if (ms === null) return "—";
  return ms < 1000 ? `${ms} ms` : ms < 60000 ? `${(ms / 1000).toFixed(1)} 秒` : `${Math.floor(ms / 60000)} 分 ${Math.floor(ms % 60000 / 1000)} 秒`;
}
export function copyLogText(entries: InternalLogEntry[]) {
  return entries.map(entry => {
    const related = [["source", entry.source], ["meeting", entry.meetingId], ["job", entry.jobId], ["operation", entry.operationId], ["session", entry.sessionId]]
      .filter(([, value]) => value).map(([key, value]) => `${key}=${value}`).join(" ");
    return `[${new Date(entry.timestamp).toISOString()}] [${entry.kind}/${entry.level}] ${entry.category} ${entry.state} ${entry.elapsedMs === null ? "" : `elapsed=${entry.elapsedMs}ms `}${entry.stream ? `${entry.stream} ` : ""}${related}\n${entry.message}${entry.message.endsWith("\n") ? "" : "\n"}`;
  }).join("\n");
}
