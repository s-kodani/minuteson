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
export const isDeletionReconfirmation = (phase: DeletionPreview["phase"] | undefined) => phase === "reconfirmation" || phase === "database_reconfirmation";
