import type { OperationProgress } from "./operation-progress-state.ts";
import { checkedStageProgress, stageLabel, stageDetail, type StageProgress } from "./stage-progress.ts";

export interface RecoveryItem {
  progress?: StageProgress | null;
  sessionId: string; meetingId: string; meetingTitle: string;
  phase: "queued" | "running" | "succeeded" | "failed" | "skipped";
  startedAtMs: number | null; error: string | null;
}
export interface RecoverySnapshot {
  runId: string; revision: number; phase: "idle" | "scanning" | "running" | "completed" | "failed";
  items: RecoveryItem[]; error: string | null;
}
export function acceptRecovery(current: RecoverySnapshot | null, input: unknown): RecoverySnapshot {
  const s = input as RecoverySnapshot;
  const optionalText = (v: unknown) => v === null || typeof v === "string";
  if (!s || typeof s.runId !== "string" || !s.runId || !Number.isSafeInteger(s.revision) || s.revision < 0
    || !["idle", "scanning", "running", "completed", "failed"].includes(s.phase) || !optionalText(s.error)
    || !Array.isArray(s.items) || new Set(s.items.map(i => i?.sessionId)).size !== s.items.length
    || s.items.some(i => !i || ![i.sessionId, i.meetingId, i.meetingTitle].every(v => typeof v === "string" && !!v)
      || !["queued", "running", "succeeded", "failed", "skipped"].includes(i.phase) || !optionalText(i.error)
      || !(i.startedAtMs === null || Number.isSafeInteger(i.startedAtMs) && i.startedAtMs >= 0)
      || i.phase === "running" && i.startedAtMs === null)
    || ["idle", "completed", "failed"].includes(s.phase) && s.items.some(i => ["queued", "running"].includes(i.phase))) throw Error("録音回復の状態を確認できません。");
  if (current && s.revision <= current.revision) return current;
  s.items.forEach(i => checkedStageProgress(i.progress));
  return s;
}
export function recoveryProgress(s: RecoverySnapshot | null): OperationProgress[] {
  if (!s) return [];
  if (s.phase === "scanning") return [{id:`recovery:${s.runId}`,target:"前回の録音",kind:"recording_recovery",state:"running",label:"回復対象を確認中",completed:null,total:null}];
  return s.items.filter(i => ["queued", "running"].includes(i.phase)).map(i => ({
    id:`recovery:${s.runId}:${i.sessionId}`,target:i.meetingTitle,meetingId:i.meetingId,
    kind:"recording_recovery",state:i.phase === "queued" ? "queued" : "running",
    label:i.phase === "queued" ? "録音の回復待ち" : stageLabel(i.progress) ?? "録音を回復中",
    completed:null,total:null,startedAtMs:i.startedAtMs ?? undefined,
    detail:stageDetail(i.progress) ?? "前回の音声保存を検証しています。",
  }));
}
export function recoveryMeetingState(s: RecoverySnapshot | null, id: string): RecoveryItem | undefined {
  const items = s?.items.filter(i => i.meetingId === id) ?? [];
  return items.find(i => i.phase === "running") ?? items.find(i => i.phase === "queued") ?? items.find(i => i.phase === "failed");
}
