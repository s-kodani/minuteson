import type { ModelDeletionEntry } from "./model-deletion-state.ts";

// Views are display data only. Requests contain the server-held confirmation ID,
// never the root, paths, hash list, recorded policy or claimed progress.
export interface ModelBodyDeletionPreview {
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
export interface ModelBodyDeletionReceipt { operationId: string; packageId: string; bodyDeleted: boolean; retained: boolean; historyRetained: true; restartRequired: boolean }
export type ModelDeletionStartupReason = "model_completed_scope_reappeared" | "model_completed_receipt_invalid" | "model_completed_scope_uninspectable";
export interface ModelBodyDeletionItem { operationId: string; state: string; packageId: string | null; modelName: string | null; reviewAllowed: boolean; errorMessage: string | null; startupReason?: ModelDeletionStartupReason | null }
export function modelDeletionStartupLabel(reason: ModelDeletionStartupReason): string {
  switch (reason) {
    case "model_completed_scope_reappeared": return "完了済みモデル削除の対象が再出現しています。自動では削除しません。現在の対象を確認し、新しい同意で解決してください。";
    case "model_completed_receipt_invalid": return "完了済みモデル削除の記録・同意履歴を検証できません。対象を保持し、通常書込みを停止しています。";
    case "model_completed_scope_uninspectable": return "完了済みモデル削除の保存領域・対象を安全に検査できません。対象を保持し、通常書込みを停止しています。";
  }
}
export interface ModelBodyDeletionListing { items: ModelBodyDeletionItem[]; writeBlocked: boolean; restartRequired: boolean; reason: string | null }
export interface ModelBodyDeletionPage { confirmationId: string; offset: number; total: number; nextOffset: number | null; entries: ModelDeletionEntry[] }
const id = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(v);
const hash = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{64}$/u.test(v);
const count = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0;
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0 && !v.includes("\0") && new TextEncoder().encode(v).length <= 4096;
const fail = (): never => { throw Error("モデル削除の状態が不正です。対象を再確認してください。"); };
const safeRelative = (v: unknown): v is string => typeof v === "string" && v.length > 0 && new TextEncoder().encode(v).length <= 4133 && !v.includes("\0") && !v.includes("\\") && !v.includes(":") && v.split("/").length <= 33
  && v.split("/").every(p => p !== "" && p !== "." && p !== ".." && new TextEncoder().encode(p).length <= 255);
function rootValid(root: string, p: Pick<ModelBodyDeletionPreview, "packageId" | "operationId">): boolean {
  if (root === p.packageId) return true;
  const prefix = `.mon-delete-model-${p.operationId}-`;
  if (!root.startsWith(prefix)) return false;
  const suffix = root.slice(prefix.length);
  if (!/^(?:g[1-9][0-9]*-)?(?:0|[1-9][0-9]*)$/u.test(suffix)) return false;
  const parts = suffix.split("-");
  return (parts.length === 1 || Number(parts[0].slice(1)) <= 32) && Number(parts.at(-1)) < 256;
}
export function assertModelBodyDeletionPreview(p: ModelBodyDeletionPreview): void {
  if (!p || ![p.confirmationId, p.operationId, p.packageId].every(id) || !hash(p.reviewHash)
    || !text(p.modelName) || !text(p.modelVersion) || !text(p.modelsRoot) || !p.modelsRoot.startsWith("/") || !p.modelsRoot.endsWith("/models")
    || p.modelsRoot.includes("\\") || p.modelsRoot.includes("//") || p.modelsRoot.split("/").some(part => part === "." || part === "..")
    || !["review", "not_started", "files", "reconfirmation", "awaiting_ack", "retained", "completed"].includes(p.phase)
    || ![p.pending, p.executeAllowed, p.keepAllowed, p.reconfirmationAllowed, p.priorDeletionCompleted].every(v => typeof v === "boolean")
    || (p.resumeReason !== null && !text(p.resumeReason)) || (p.executeAllowed && p.resumeReason !== null)
    || (p.includeUnregisteredRecorded !== null && typeof p.includeUnregisteredRecorded !== "boolean")
    || ![p.pathCount, p.processedPaths, p.fileCount, p.missingCount, p.changedCount, p.unregisteredCount, p.bytes, p.profileReferences, p.jobReferences, p.activeJobs].every(count)
    || p.pathCount > 256 || p.processedPaths > p.pathCount || p.fileCount > p.pathCount || p.missingCount > p.pathCount || p.changedCount > p.fileCount || p.unregisteredCount > p.pathCount
    || p.missingCount + p.changedCount + p.unregisteredCount > p.pathCount || (p.executeAllowed && p.activeJobs > 0)
    || p.pending !== ["not_started", "files", "reconfirmation", "awaiting_ack"].includes(p.phase)
    || p.keepAllowed !== ["not_started", "retained"].includes(p.phase)
    || (["review", "not_started", "retained", "reconfirmation"].includes(p.phase) && p.processedPaths !== 0)
    || (["awaiting_ack", "completed"].includes(p.phase) && p.processedPaths !== p.pathCount)
    || (["retained", "completed"].includes(p.phase) && p.executeAllowed)
    || (p.phase === "review" && p.priorDeletionCompleted)
    || (p.reconfirmationAllowed && (!["files", "awaiting_ack"].includes(p.phase) || p.executeAllowed))
    || (["not_started", "files", "awaiting_ack", "completed"].includes(p.phase) ? p.includeUnregisteredRecorded === null : p.includeUnregisteredRecorded !== null)
    || (p.phase === "reconfirmation" ? (!count(p.previousProcessedPaths) || !count(p.previousPathCount) || p.previousPathCount > 256 || p.previousProcessedPaths > p.previousPathCount) : (p.previousProcessedPaths !== null || p.previousPathCount !== null))
    || !Array.isArray(p.reservationRoots) || p.reservationRoots.length > 64 || !p.reservationRoots.every((r, i) => typeof r === "string" && rootValid(r, p) && (i === 0 || p.reservationRoots[i - 1] < r))
    || (["review", "not_started", "retained"].includes(p.phase) && p.reservationRoots.length > 0)) fail();
}
export function assertModelBodyDeletionReceipt(r: ModelBodyDeletionReceipt, p: ModelBodyDeletionPreview | null | undefined): void {
  if (!r || !p || !id(r.operationId) || !id(r.packageId) || r.operationId !== p.operationId || r.packageId !== p.packageId
    || typeof r.bodyDeleted !== "boolean" || r.retained !== !r.bodyDeleted || r.historyRetained !== true || typeof r.restartRequired !== "boolean"
    || p.phase !== (r.bodyDeleted ? "completed" : "retained") || p.pending || p.executeAllowed) fail();
}
export function assertModelBodyDeletionListing(l: ModelBodyDeletionListing): void {
  if (!l || !Array.isArray(l.items) || l.items.length > 100 || typeof l.writeBlocked !== "boolean" || typeof l.restartRequired !== "boolean"
    || (l.reason !== null && !text(l.reason)) || l.writeBlocked !== (l.reason !== null) || (l.restartRequired && !l.writeBlocked)) fail();
  for (const item of l.items) if (!item || !id(item.operationId) || !text(item.state) || typeof item.reviewAllowed !== "boolean"
    || (item.startupReason != null && (!["model_completed_scope_reappeared", "model_completed_receipt_invalid", "model_completed_scope_uninspectable"].includes(item.startupReason) || item.state !== "completed" || !l.writeBlocked))
    || (item.packageId !== null && !id(item.packageId)) || (item.modelName !== null && !text(item.modelName)) || (item.errorMessage !== null && !text(item.errorMessage))
    || (item.reviewAllowed ? item.packageId === null || item.modelName === null || item.errorMessage !== null : item.packageId !== null || item.modelName !== null || item.errorMessage === null)) fail();
  if (new Set(l.items.map(i => i.operationId)).size !== l.items.length) fail();
}
export function assertModelBodyDeletionPage(page: ModelBodyDeletionPage, p: ModelBodyDeletionPreview, offset: number): void {
  assertModelBodyDeletionPreview(p);
  if (!page || page.confirmationId !== p.confirmationId || page.offset !== offset || !count(offset) || offset % 50 !== 0 || offset > p.pathCount
    || page.total !== p.pathCount || !Array.isArray(page.entries) || page.entries.length !== Math.min(50, p.pathCount - offset)
    || page.nextOffset !== (offset + page.entries.length < p.pathCount ? offset + page.entries.length : null)) fail();
  const roots = [p.packageId, ...p.reservationRoots];
  let previous: Uint8Array | null = null;
  for (const e of page.entries) {
    if (!e || !safeRelative(e.relativePath) || !roots.some(r => e.relativePath === r || e.relativePath.startsWith(r + "/"))
      || ![e.directory, e.registered, e.present, e.changed].every(v => typeof v === "boolean") || (!e.present && !e.registered)
      || (e.directory || !e.present ? e.bytes !== null || e.sha256 !== null || e.changed : !count(e.bytes) || !hash(e.sha256) || (e.changed && !e.registered))) fail();
    if (e.relativePath.split("/")[0] !== p.packageId && (e.registered || e.changed)) fail();
    const current = new TextEncoder().encode(e.relativePath);
    if (previous) {
      let order = previous.length - current.length;
      for (let i = 0; i < Math.min(previous.length, current.length); i++) if (previous[i] !== current[i]) { order = previous[i] - current[i]; break; }
      if (order >= 0) fail();
    }
    previous = current;
  }
  if (new Set(page.entries.map(e => e.relativePath)).size !== page.entries.length) fail();
}
