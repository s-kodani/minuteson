export type ModelDeletionStartupReason = "model_completed_scope_reappeared" | "model_completed_receipt_invalid" | "model_completed_scope_uninspectable";
export interface ModelBodyDeletionItem { operationId: string; state: string; packageId: string | null; modelName: string | null; reviewAllowed: boolean; errorMessage: string | null; startupReason?: ModelDeletionStartupReason | null }
export interface ModelBodyDeletionListing { items: ModelBodyDeletionItem[]; writeBlocked: boolean; restartRequired: boolean; reason: string | null }

export function modelDeletionStartupLabel(reason: ModelDeletionStartupReason): string {
  switch (reason) {
    case "model_completed_scope_reappeared": return "完了済みモデル削除の対象が再出現しています。自動では削除しません。現在の対象を確認し、新しい同意で解決してください。";
    case "model_completed_receipt_invalid": return "完了済みモデル削除の記録・同意履歴を検証できません。対象を保持し、通常書込みを停止しています。";
    case "model_completed_scope_uninspectable": return "完了済みモデル削除の保存領域・対象を安全に検査できません。対象を保持し、通常書込みを停止しています。";
  }
}

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
export interface ModelDeletionEntry {
  relativePath: string; directory: boolean; registered: boolean; present: boolean; bytes: number | null; sha256: string | null; changed: boolean;
}
export interface ModelDeletionPage { confirmationId: string; offset: number; total: number; nextOffset: number | null; entries: ModelDeletionEntry[] }
const count = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0;
const fail = (): never => { throw Error("モデル削除の状態が不正です。対象を再確認してください。"); };
const reservation = (root: string, operation: string) => {
  const prefix = `.mon-delete-model-${operation}-`;
  if (!root.startsWith(prefix)) return false;
  const suffix = root.slice(prefix.length), match = /^(?:g([1-9][0-9]*)-)?(0|[1-9][0-9]*)$/u.exec(suffix);
  return !!match && (match[1] === undefined || Number(match[1]) <= 32) && Number(match[2]) < 256;
};
export function assertModelDeletionPreview(p: ModelDeletionPreview): void {
  if (!p || ![p.confirmationId, p.operationId, p.packageId].every(id) || !/^[0-9a-f]{64}$/u.test(p.reviewHash)
    || !text(p.modelName) || !text(p.modelVersion) || !text(p.modelsRoot) || !p.modelsRoot.startsWith("/")
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
export function assertModelDeletionPage(page: ModelDeletionPage, p: ModelDeletionPreview, offset: number): void {
  assertModelDeletionPreview(p);
  if (!page || page.confirmationId !== p.confirmationId || page.offset !== offset || page.total !== p.pathCount
    || !count(offset) || offset % 50 !== 0 || offset > p.pathCount || !Array.isArray(page.entries)
    || page.entries.length !== Math.min(50, p.pathCount - offset) || page.nextOffset !== (offset + page.entries.length < p.pathCount ? offset + page.entries.length : null)) fail();
  for (const e of page.entries) {
    if (!e || !text(e.relativePath) || new TextEncoder().encode(e.relativePath).length > 4096 || e.relativePath.includes("\\")
      || e.relativePath.split("/").some(part => !part || part === "." || part === "..")
      || ![p.packageId, ...p.reservationRoots].some(root => e.relativePath === root || e.relativePath.startsWith(root + "/"))
      || ![e.directory, e.registered, e.present, e.changed].every(v => typeof v === "boolean")
      || (!e.present && !e.registered) || (e.directory || !e.present ? e.bytes !== null || e.sha256 !== null || e.changed : !count(e.bytes) || typeof e.sha256 !== "string" || !/^[0-9a-f]{64}$/u.test(e.sha256))
      || (e.changed && !e.registered)) fail();
  }
  if (new Set(page.entries.map(e => e.relativePath)).size !== page.entries.length) fail();
}
export const modelDeletionPhaseLabel = (phase: ModelDeletionPreview["phase"]) => ({ review: "削除前の確認", not_started: "ファイル削除は未開始", files: "ファイル削除の途中", reconfirmation: "変更後の残り範囲への再同意待ち", awaiting_ack: "完了応答を未確認", retained: "保持して中止", completed: "削除完了" })[phase];
