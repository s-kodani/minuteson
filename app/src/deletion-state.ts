export interface DeletionPreview {
  scopeMode?: "manifest" | "directory"; targetDirectory?: string;
  confirmationId: string; operationId: string; meetingId: string; title: string;
  phase: "review" | "not_started" | "files" | "reconfirmation" | "database_files" | "database_reconfirmation" | "database" | "awaiting_ack" | "retained" | "completed";
  pending: boolean; executeAllowed: boolean; resumeReason: string | null; keepAllowed: boolean;
  pathCount: number; processedPaths: number; fileCount: number; missingCount: number;
  changedCount: number; unregisteredCount: number; bytes: number; rowCount: number; rowPhaseCommitted: boolean;
  reconfirmationAllowed?: boolean; previousProcessedPaths?: number | null; previousPathCount?: number | null;
  reservationRoots?: string[];
}
export interface DeletionReceipt { operationId: string; meetingId: string; deleted: boolean; retained: boolean; restartRequired: boolean }
export interface DeletionItem { operationId: string; state: string; meetingId: string | null; errorCode: string | null }
export interface DeletionListing { items: DeletionItem[]; deletingMeetingIds: string[]; writeBlocked: boolean; restartRequired: boolean; reason: string | null }
export interface DeletionPath { relativePath: string; directory: boolean; registered: boolean; missing: boolean; changed: boolean; stageHint: string | null; bytes: number | null }
export interface DeletionPathPage { confirmationId: string; offset: number; total: number; nextOffset: number | null; entries: DeletionPath[] }
const uuid = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(v);
const count = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0;
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0 && !v.includes("\0");
const fail = (): never => { throw Error("削除の状態が不正です。対象を再確認してください。"); };
const validReservationRoot = (path: string, operationId: string) => {
  const prefix = `meetings/.mon-delete-${operationId}-`;
  if (!path.startsWith(prefix)) return false;
  const suffix = path.slice(prefix.length);
  if (!/^(?:r[1-9][0-9]*-)?(?:0|[1-9][0-9]*)$/u.test(suffix)) return false;
  const parts = suffix.split("-");
  return (parts.length === 1 || Number(parts[0].slice(1)) <= 32) && Number(parts.at(-1)) < 200000;
};
const validReservationRoots = (roots: unknown, operationId: string): roots is string[] => Array.isArray(roots) && roots.length <= 256
  && roots.every((p, i) => typeof p === "string" && validReservationRoot(p, operationId) && (i === 0 || roots[i - 1] < p));
export function assertDeletionPreview(p: DeletionPreview): void {
  if (p?.scopeMode !== undefined && !["manifest", "directory"].includes(p.scopeMode)) fail();
  if (p?.scopeMode === "directory" && (typeof p.targetDirectory !== "string" || !/^(?:\/|[A-Za-z]:[\\/]|\\\\)/u.test(p.targetDirectory) || p.targetDirectory.includes("\0"))) fail();
  if (!p || ![p.confirmationId, p.operationId, p.meetingId].every(uuid) || !text(p.title)
    || !["review", "not_started", "files", "reconfirmation", "database_files", "database_reconfirmation", "database", "awaiting_ack", "retained", "completed"].includes(p.phase)
    || ![p.pending, p.executeAllowed, p.keepAllowed, p.rowPhaseCommitted].every(v => typeof v === "boolean")
    || (p.resumeReason !== null && !text(p.resumeReason))
    || (p.executeAllowed && p.resumeReason !== null)
    || (p.reservationRoots !== undefined && !validReservationRoots(p.reservationRoots, p.operationId))
    || (["review", "not_started", "retained"].includes(p.phase) && !!p.reservationRoots?.length)
    || ![p.pathCount, p.processedPaths, p.fileCount, p.missingCount, p.changedCount, p.unregisteredCount, p.bytes, p.rowCount].every(count)
    || p.pathCount > 200000 || p.processedPaths > p.pathCount || p.fileCount > p.pathCount
    || p.missingCount + p.changedCount + p.unregisteredCount > p.fileCount
    || (p.pending !== ["not_started", "files", "reconfirmation", "database_files", "database_reconfirmation", "database", "awaiting_ack"].includes(p.phase))
    || (p.keepAllowed !== ["not_started", "retained"].includes(p.phase))
    || (["review", "not_started", "retained"].includes(p.phase) && (p.processedPaths !== 0 || p.rowPhaseCommitted))
    || (["files", "reconfirmation"].includes(p.phase) && p.rowPhaseCommitted)
    || (p.reconfirmationAllowed !== undefined && typeof p.reconfirmationAllowed !== "boolean")
    || (p.reconfirmationAllowed && (!["files", "database_files", "database", "awaiting_ack"].includes(p.phase) || p.executeAllowed))
    || (isDeletionReconfirmation(p.phase) ? (!count(p.previousProcessedPaths) || !count(p.previousPathCount) || p.previousPathCount > 200000 || p.previousProcessedPaths > p.previousPathCount || p.processedPaths !== 0 || p.reconfirmationAllowed !== false) : (p.previousProcessedPaths != null || p.previousPathCount != null))
    || (["database", "awaiting_ack", "completed"].includes(p.phase) && p.processedPaths !== p.pathCount)
    || (["awaiting_ack", "completed"].includes(p.phase) && !p.rowPhaseCommitted)
    || (["retained", "completed"].includes(p.phase) && p.executeAllowed)) fail();
}
export function assertDeletionReceipt(r: DeletionReceipt, p: DeletionPreview | null | undefined): void {
  if (!r || !p || ![r.operationId, r.meetingId].every(uuid) || typeof r.deleted !== "boolean"
    || r.retained !== !r.deleted || typeof r.restartRequired !== "boolean"
    || r.operationId !== p.operationId || r.meetingId !== p.meetingId
    || p.phase !== (r.deleted ? "completed" : "retained") || p.pending || p.executeAllowed) fail();
}
export function assertDeletionListing(l: DeletionListing): void {
  if (!l || !Array.isArray(l.items) || l.items.length > 100 || !Array.isArray(l.deletingMeetingIds)
    || !l.deletingMeetingIds.every(uuid) || new Set(l.deletingMeetingIds).size !== l.deletingMeetingIds.length
    || typeof l.writeBlocked !== "boolean" || typeof l.restartRequired !== "boolean"
    || (l.reason !== null && !text(l.reason)) || l.writeBlocked !== (l.reason !== null)
    || (l.restartRequired && !l.writeBlocked)) fail();
  for (const i of l.items) if (!i || !uuid(i.operationId) || !text(i.state)
    || (i.meetingId !== null && !uuid(i.meetingId)) || (i.errorCode !== null && !text(i.errorCode))) fail();
  if (new Set(l.items.map(i => i.operationId)).size !== l.items.length) fail();
}
export function assertDeletionPathPage(page: DeletionPathPage, confirmation: string, offset: number, total: number, meetingId: string, reservationRoots: string[] = [], operationId?: string): void {
  if (!page || page.confirmationId !== confirmation || !uuid(confirmation) || page.offset !== offset || page.total !== total
    || !count(offset) || offset % 50 !== 0 || !count(total) || total > 200000 || offset > total
    || !Array.isArray(page.entries) || page.entries.length !== Math.min(50, total - offset)
    || page.nextOffset !== (offset + page.entries.length < total ? offset + page.entries.length : null)) fail();
  if (!uuid(meetingId) || !Array.isArray(reservationRoots) || (reservationRoots.length > 0 && (!uuid(operationId) || !validReservationRoots(reservationRoots, operationId)))) fail();
  const prefix = `meetings/${meetingId}`;
  for (const e of page.entries) {
    const reserved = e && typeof e.relativePath === "string" && reservationRoots.some(r => e.relativePath === r || e.relativePath.startsWith(r + "/"));
    if (!e || !text(e.relativePath) || !(e.relativePath === prefix || e.relativePath.startsWith(prefix + "/") || reserved) || new TextEncoder().encode(e.relativePath).length > 256
      || e.relativePath.includes("\\") || e.relativePath.split("/").some(p => !p || p === "." || p === "..")
      || ![e.directory, e.registered, e.missing, e.changed].every(v => typeof v === "boolean")
      || (e.stageHint !== null && !text(e.stageHint)) || (e.bytes !== null && !count(e.bytes))
      || (e.directory && (e.registered || e.changed || e.bytes !== null))
      || (!e.directory && e.missing !== (e.bytes === null)) || (e.changed && (!e.registered || e.missing))) fail();
    if (reserved && (e.registered || e.changed || e.stageHint !== null || (!e.directory && reservationRoots.includes(e.relativePath)))) fail();
  }
  if (new Set(page.entries.map(e => e.relativePath)).size !== page.entries.length) fail();
}
export const isDeletionReconfirmation = (phase: DeletionPreview["phase"] | undefined) => phase === "reconfirmation" || phase === "database_reconfirmation";
export const deletionAttentionLabel = (code: string | null): string | null => {
  switch (code) {
    case "completed_scope_reappeared": return "完了済み削除の対象が再出現しました。自動削除せず保持しています。新しい範囲を確認してください。";
    case "completed_receipt_invalid": return "完了済み削除の記録を検証できません。対象を保持し、通常書込みを停止しています。";
    case "completed_scope_uninspectable": return "完了済み削除の対象を安全に確認できません。リンクなどを採用せず保持しています。";
    default: return null;
  }
};
export const deletionPhaseLabel = (phase: DeletionPreview["phase"]) => ({ review: "削除前の確認", not_started: "確認記録を保存済み・ファイル削除は未開始", files: "ファイル削除の途中", reconfirmation: "変更後の残り範囲への再確認待ち", database_files: "DB段階を保持した残りファイルの削除", database_reconfirmation: "DB段階を保持した残り範囲への再確認待ち", database: "DB削除・後処理の途中", awaiting_ack: "後処理の完了応答を未確認", retained: "未開始のまま保持して中止", completed: "削除完了を確認済み" })[phase];
