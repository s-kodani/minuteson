import { displayNativePathsInText } from "./native-path.ts";

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
  meeting_open_diagnostics: "会議を開くときの診断",
  background_read: "自動確認の診断",
  recording_diagnostics: "マイク・PC音声の中断・再開履歴",
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
const entryFields = ["id", "timestamp", "sessionId", "kind", "level", "category", "meetingId", "jobId", "operationId", "state", "elapsedMs", "message", "stream", "source"] as const satisfies readonly (keyof InternalLogEntry)[];
export function mergeLogEntries(current: InternalLogEntry[], incoming: InternalLogEntry[], direction: "initial" | "older" | "newer") {
  if (direction !== "initial" && !incoming.length) return { entries: current, trimmed: false };
  const previous = new Map(current.map(entry => [entry.id, entry]));
  const unique = direction === "initial" ? new Map<number, InternalLogEntry>() : new Map(previous);
  incoming.forEach(entry => {
    const prior = previous.get(entry.id);
    unique.set(entry.id, prior && entryFields.every(field => prior[field] === entry[field]) ? prior : entry);
  });
  const ordered = [...unique.values()].sort((a, b) => a.id - b.id);
  const entries = direction === "older" ? ordered.slice(0, 1000) : ordered.slice(-1000);
  // Native pages deserialize into new objects even when nothing has changed.
  // Keep both the collection and unchanged rows stable for the mounted viewer.
  const unchanged = entries.length === current.length && entries.every((entry, index) => entry === current[index]);
  return { entries: unchanged ? current : entries, trimmed: ordered.length > 1000 };
}
export function logElapsed(ms: number | null) {
  if (ms === null) return "—";
  return ms < 1000 ? `${ms} ms` : ms < 60000 ? `${(ms / 1000).toFixed(1)} 秒` : `${Math.floor(ms / 60000)} 分 ${Math.floor(ms % 60000 / 1000)} 秒`;
}
export function copyLogText(entries: InternalLogEntry[]) {
  return entries.map(entry => {
    const related = [["source", entry.source], ["meeting", entry.meetingId], ["job", entry.jobId], ["operation", entry.operationId], ["session", entry.sessionId]]
      .filter(([, value]) => value).map(([key, value]) => `${key}=${value}`).join(" ");
    return displayNativePathsInText(`[${new Date(entry.timestamp).toISOString()}] [${entry.kind}/${entry.level}] ${entry.category} ${entry.state} ${entry.elapsedMs === null ? "" : `elapsed=${entry.elapsedMs}ms `}${entry.stream ? `${entry.stream} ` : ""}${related}\n${entry.message}${entry.message.endsWith("\n") ? "" : "\n"}`);
  }).join("\n");
}
