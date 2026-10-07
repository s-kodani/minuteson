export type RecordingDecisionPhase = "idle" | "confirming" | "stopping" | "completed" | "cancelled" | "failed";
export interface RecordingDecision {
  revision: number; id: string | null; phase: RecordingDecisionPhase;
  sessionId: string | null; meetingId: string | null; meetingTitle: string | null;
  origin: "main" | "recorder" | null; preparing: boolean; quitAfterSave: boolean; errorMessage: string | null;
}
export const recordingDecisionVisible = (value: RecordingDecision | null) => !!value && ["confirming", "stopping", "failed"].includes(value.phase);
export function assertRecordingDecision(value: RecordingDecision): void {
  const optional = (v: unknown) => v === null || (typeof v === "string" && v.length > 0);
  if (typeof value?.quitAfterSave !== "boolean" || (value.quitAfterSave && value.origin !== "main")) throw Error("録音終了の目的を検証できません");
  if (!value || !Number.isSafeInteger(value.revision) || value.revision < 0 || !["idle", "confirming", "stopping", "completed", "cancelled", "failed"].includes(value.phase) || typeof value.preparing !== "boolean" || ![null,"main","recorder"].includes(value.origin) || !optional(value.id) || !optional(value.sessionId) || !optional(value.meetingId) || !optional(value.meetingTitle) || !optional(value.errorMessage)) throw Error("録音確認の状態を検証できません");
  if (value.phase === "idle" ? value.id !== null || value.sessionId !== null || value.meetingId !== null || value.origin !== null : !value.id || !value.sessionId || !value.meetingId || !value.origin || !value.meetingTitle) throw Error("録音確認の対象を検証できません");
}
export function acceptRecordingDecision(current: RecordingDecision | null, next: RecordingDecision) {
  assertRecordingDecision(next);
  if (current && next.revision < current.revision) return current;
  if (current && next.revision === current.revision && (Object.keys(current) as (keyof RecordingDecision)[]).some(key => next[key] !== current[key])) throw Error(`同じ版の録音確認が変わっています (${next.revision}: ${current.phase} → ${next.phase})`);
  return next;
}
