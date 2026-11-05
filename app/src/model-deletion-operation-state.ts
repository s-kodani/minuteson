import { nativeAbsolutePath } from "./native-path.ts";
export type ModelDeletionStartupReason = "model_completed_scope_reappeared" | "model_completed_receipt_invalid" | "model_completed_scope_uninspectable";
export interface ModelBodyDeletionItem { operationId: string; state: string; packageId: string | null; modelName: string | null; reviewAllowed: boolean; errorMessage: string | null; startupReason?: ModelDeletionStartupReason | null }
export interface ModelBodyDeletionListing { items: ModelBodyDeletionItem[]; writeBlocked: boolean; restartRequired: boolean; reason: string | null }

const id = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(v);
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0 && !v.includes("\0") && new TextEncoder().encode(v).length <= 4096;
export function assertModelBodyDeletionListing(l: ModelBodyDeletionListing): void {
  const fail = (): never => { throw Error("モデル削除の状態が不正です。対象を再確認してください。"); };
  if (!l || !Array.isArray(l.items) || l.items.length > 100 || typeof l.writeBlocked !== "boolean" || typeof l.restartRequired !== "boolean"
    || (l.reason !== null && !text(l.reason)) || l.writeBlocked !== (l.reason !== null) || (l.restartRequired && !l.writeBlocked)) fail();
  for (const item of l.items) if (!item || !id(item.operationId) || !text(item.state) || typeof item.reviewAllowed !== "boolean"
    || (item.startupReason != null && (!["model_completed_scope_reappeared", "model_completed_receipt_invalid", "model_completed_scope_uninspectable"].includes(item.startupReason) || item.state !== "completed" || !l.writeBlocked))
    || (item.packageId !== null && !id(item.packageId)) || (item.modelName !== null && !text(item.modelName)) || (item.errorMessage !== null && !text(item.errorMessage))
    || (item.reviewAllowed ? item.packageId === null || item.modelName === null || item.errorMessage !== null : item.packageId !== null || item.modelName !== null || item.errorMessage === null)) fail();
  if (new Set(l.items.map(i => i.operationId)).size !== l.items.length) fail();
}


export interface ModelDeletionPreview {
  scopeMode?: "manifest" | "directory"; targetDirectory?: string;
  confirmationId: string; operationId: string; packageId: string; reviewHash: string;
  modelName: string; modelVersion: string; modelsRoot: string;
  phase: "review" | "not_started" | "files" | "reconfirmation" | "awaiting_ack" | "retained" | "completed";
  pending: boolean; executeAllowed: boolean; resumeReason: string | null; keepAllowed: boolean;
  reconfirmationAllowed: boolean; includeUnregisteredRecorded: boolean | null;
  pathCount: number; processedPaths: number; fileCount: number; missingCount: number;
  changedCount: number; unregisteredCount: number; bytes: number;
  profileReferences: number; jobReferences: number; activeJobs: number;
  previousProcessedPaths: number | null; previousPathCount: number | null;
  priorDeletionCompleted: boolean; reservationRoots: string[];
}
export interface ModelDeletionReceipt {
  operationId: string; packageId: string; bodyDeleted: boolean; retained: boolean; historyRetained: true; restartRequired: boolean;
}
const count = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0;
const fail = (): never => { throw Error("モデル削除の状態が不正です。対象を再確認してください。"); };
const reservation = (root: string, operation: string) => {
  const prefix = `.mon-delete-model-${operation}-`;
  if (!root.startsWith(prefix)) return false;
  const suffix = root.slice(prefix.length), match = /^(?:g([1-9][0-9]*)-)?(0|[1-9][0-9]*)$/u.exec(suffix);
  return !!match && (match[1] === undefined || Number(match[1]) <= 32) && Number(match[2]) < 256;
};
export function assertModelDeletionPreview(p: ModelDeletionPreview): void {
  if (p?.scopeMode !== undefined && !["manifest", "directory"].includes(p.scopeMode)) fail();
  if (p?.scopeMode === "directory" && (typeof p.targetDirectory !== "string" || !/^(?:\/|[A-Za-z]:[\\/]|\\\\)/u.test(p.targetDirectory) || p.targetDirectory.includes("\0"))) fail();
  if (!p || ![p.confirmationId, p.operationId, p.packageId].every(id) || !/^[0-9a-f]{64}$/u.test(p.reviewHash)
    || !text(p.modelName) || !text(p.modelVersion) || !nativeAbsolutePath(p.modelsRoot)
    || !["review", "not_started", "files", "reconfirmation", "awaiting_ack", "retained", "completed"].includes(p.phase)
    || ![p.pending, p.executeAllowed, p.keepAllowed, p.reconfirmationAllowed, p.priorDeletionCompleted].every(v => typeof v === "boolean")
    || (p.resumeReason !== null && !text(p.resumeReason)) || (p.executeAllowed && p.resumeReason !== null)
    || ![p.pathCount, p.processedPaths, p.fileCount, p.missingCount, p.changedCount, p.unregisteredCount, p.bytes, p.profileReferences, p.jobReferences, p.activeJobs].every(count)
    || p.pathCount > 256 || p.processedPaths > p.pathCount || p.fileCount > p.pathCount || p.missingCount > p.pathCount || p.changedCount > p.fileCount || p.unregisteredCount > p.pathCount
    || p.pending !== ["not_started", "files", "reconfirmation", "awaiting_ack"].includes(p.phase)
    || p.keepAllowed !== ["not_started", "retained"].includes(p.phase)
    || (["review", "not_started", "retained", "reconfirmation"].includes(p.phase) && p.processedPaths !== 0)
    || (["awaiting_ack", "completed"].includes(p.phase) && p.processedPaths !== p.pathCount)
    || (["retained", "completed"].includes(p.phase) && p.executeAllowed)
    || (p.reconfirmationAllowed && (p.phase !== "files" && p.phase !== "awaiting_ack" || p.executeAllowed))
    || (p.phase === "reconfirmation" ? !count(p.previousProcessedPaths) || !count(p.previousPathCount) || p.previousPathCount > 256 || p.previousProcessedPaths > p.previousPathCount : p.previousProcessedPaths !== null || p.previousPathCount !== null)
    || (["review", "reconfirmation", "retained"].includes(p.phase) ? p.includeUnregisteredRecorded !== null : typeof p.includeUnregisteredRecorded !== "boolean")
    // Native review scopes include the original package root as well as reservations.
    || !Array.isArray(p.reservationRoots) || p.reservationRoots.length > 256 || p.reservationRoots.some((r, i) => typeof r !== "string" || (r !== p.packageId && !reservation(r, p.operationId)) || i > 0 && p.reservationRoots[i - 1] >= r)) fail();
}
export function assertModelDeletionReceipt(r: ModelDeletionReceipt, p: ModelDeletionPreview | null | undefined): void {
  if (!r || !p || ![r.operationId, r.packageId].every(id) || r.operationId !== p.operationId || r.packageId !== p.packageId
    || typeof r.bodyDeleted !== "boolean" || r.retained !== !r.bodyDeleted || r.historyRetained !== true || typeof r.restartRequired !== "boolean"
    || p.phase !== (r.bodyDeleted ? "completed" : "retained") || p.pending || p.executeAllowed) fail();
}
