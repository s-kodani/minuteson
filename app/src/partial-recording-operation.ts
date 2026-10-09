import type { DataRequest, DataSnapshot } from "./data-operation-state.ts";
export interface PartialChoice { sessionId: string; revision: number }
export interface PartialPending { request: DataRequest; revision: number }
export interface PartialCancel { token: string; revision: number }
export function partialCancelObserved(s: DataSnapshot | null, pending: PartialCancel): boolean {
  return !!s && s.kind === "partial_recording" && s.token === pending.token && s.revision > pending.revision &&
    (["cancelling", "discarding", "cancelled", "failed", "attention"].includes(s.phase) || s.phase === "completed" && !!s.partialRecordingReceipt);
}
export function partialChoiceObserved(s: DataSnapshot | null, choice: PartialChoice): boolean {
  return !!s && s.kind === "partial_recording" && s.revision > choice.revision &&
    (s.partialRecordingPreview?.summary.sessionId === choice.sessionId ||
      s.phase === "failed" && !s.partialRecordingPreview);
}
export function partialRequestObserved(s: DataSnapshot | null, pending: PartialPending): boolean {
  return !!s && s.kind === "partial_recording" && s.token === pending.request.token && s.revision > pending.revision &&
    (s.busy || ["completed", "failed", "cancelled", "attention"].includes(s.phase) ||
      s.phase === "ready" && s.partialRecordingPreview?.confirmationId !== pending.request.confirmationId);
}
