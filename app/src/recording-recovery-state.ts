import type { OperationProgress } from "./operation-progress-state.ts";
import { recordingSaveProgress } from "./operation-progress-state.ts";
import type { RecordingSnapshot } from "./session.ts";
import { checkedStageProgress, stageLabel, stageDetail, type StageProgress } from "./stage-progress.ts";

export interface RecoveryItem {
  progress?: StageProgress | null;
  sessionId: string; meetingId: string; meetingTitle: string;
  phase: "queued" | "running" | "succeeded" | "failed" | "skipped";
  startedAtMs: number | null; error: string | null;
  warning?: string | null;
  noAudio?: boolean;
}
export interface RecoverySnapshot {
  runId: string; revision: number; phase: "idle" | "scanning" | "running" | "completed" | "failed";
  origin: "stop" | "startup" | "retry";
  targetSessionId: string | null;
  items: RecoveryItem[]; error: string | null;
}
export function acceptRecovery(current: RecoverySnapshot | null, input: unknown): RecoverySnapshot {
  const s = input as RecoverySnapshot;
  const optionalText = (v: unknown) => v === null || typeof v === "string";
  if (!s || typeof s.runId !== "string" || !s.runId || !Number.isSafeInteger(s.revision) || s.revision < 0
    || !["stop", "startup", "retry"].includes(s.origin)
    || !(s.targetSessionId === null || typeof s.targetSessionId === "string" && !!s.targetSessionId)
    || s.origin !== "startup" && s.targetSessionId === null
    || !["idle", "scanning", "running", "completed", "failed"].includes(s.phase) || !optionalText(s.error)
    || !Array.isArray(s.items) || new Set(s.items.map(i => i?.sessionId)).size !== s.items.length
    || s.items.some(i => !i || ![i.sessionId, i.meetingId, i.meetingTitle].every(v => typeof v === "string" && !!v)
      || !["queued", "running", "succeeded", "failed", "skipped"].includes(i.phase) || !optionalText(i.error)
      || i.warning !== undefined && !optionalText(i.warning)
      || i.noAudio !== undefined && typeof i.noAudio !== "boolean"
      || !(i.startedAtMs === null || Number.isSafeInteger(i.startedAtMs) && i.startedAtMs >= 0)
      || i.phase === "running" && i.startedAtMs === null)
    || ["idle", "completed", "failed"].includes(s.phase) && s.items.some(i => ["queued", "running"].includes(i.phase))) throw Error("録音保存の状態を確認できません。");
  if (current && s.revision <= current.revision) return current;
  s.items.forEach(i => checkedStageProgress(i.progress));
  return s;
}
export function recoveryProgress(s: RecoverySnapshot | null): OperationProgress[] {
  if (!s) return [];
  if (s.phase === "scanning") {
    const item = s.items.find(i => i.sessionId === s.targetSessionId);
    return [{id:s.targetSessionId ? `recording_save:${s.targetSessionId}` : `recording_save:scan:${s.runId}`,
      target:item?.meetingTitle ?? (s.origin === "startup" ? "前回の録音" : "録音した会議"), meetingId:item?.meetingId,
      kind:"recording_save",state:"running",label:"保存対象を確認中",completed:null,total:null,
      detail:s.origin === "retry" ? "録音の保存を再試行しています。" : s.origin === "startup" ? "中断した録音の保存対象を確認しています。" : "停止した録音を確認しています。"}];
  }
  return s.items.filter(i => ["queued", "running"].includes(i.phase)).map(i => ({
    id:`recording_save:${i.sessionId}`,target:i.meetingTitle,meetingId:i.meetingId,
    kind:"recording_save",state:i.phase === "queued" ? "queued" : "running",
    label:i.phase === "queued" ? "録音の保存待ち" : stageLabel(i.progress) ?? "録音を保存中",
    completed:null,total:null,startedAtMs:i.startedAtMs ?? undefined,
    detail:[s.origin === "retry" ? "録音の保存を再試行しています。" : s.origin === "startup" ? "中断した録音を確認して保存しています。" : undefined, stageDetail(i.progress)].filter(Boolean).join(" ") || "録音の保存完了を確認しています。",
  }));
}
/** One display owner joins recorder and saving snapshots. Terminal save evidence
 * masks a delayed recorder update; a retry is admitted only by a newer snapshot. */
export function recordingSaveJobs(recording: RecordingSnapshot | null, recovery: RecoverySnapshot | null, title?: string): OperationProgress[] {
  const outer = recordingSaveProgress(recording, title);
  const rows = new Map<string, OperationProgress>();
  if (outer && !recovery?.items.some(i => i.sessionId === recording?.sessionId && ["succeeded", "failed", "skipped"].includes(i.phase))) rows.set(outer.id, outer);
  for (const item of recoveryProgress(recovery)) {
    const prior = rows.get(item.id);
    rows.set(item.id, prior && !item.meetingId ? {...item, target:prior.target, meetingId:prior.meetingId} : item);
  }
  return [...rows.values()];
}
export function recoveryMeetingState(s: RecoverySnapshot | null, id: string): RecoveryItem | undefined {
  const items = s?.items.filter(i => i.meetingId === id) ?? [];
  return items.find(i => i.phase === "running") ?? items.find(i => i.phase === "queued") ?? items.find(i => i.phase === "failed");
}
