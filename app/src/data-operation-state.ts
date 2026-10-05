import { assertRecoveryPreview, assertRecoveryReceipt, type RecoveryPreview, type RecoveryReceipt } from "./recovery-state.ts";
import { assertDeletionPreview, assertDeletionReceipt, type DeletionPreview, type DeletionReceipt } from "./deletion-state.ts";
export type DataKind = "backup" | "restore" | "recovery" | "relocation" | "deletion";
export interface MovePreview { confirmationId: string; operationId: string; source: string; destination: string; meetingCount: number; fileCount: number | null; requiredBytes: number; availableBytes: number; pending: boolean; resumeAllowed: boolean; resumeReason: string | null; copiedPaths: number | null; expectedPaths: number | null }
export interface MoveReceipt { operationId: string; source: string; destination: string; restartRequired: boolean; retained: boolean }
export interface BackupPreview { confirmationId: string; destination: string; replacesExisting: boolean }
export interface RestorePreview {
  confirmationId: string; batchId: string; archiveId: string; meetingTitles: string[];
  requiredBytes: number; availableBytes: number;
  vocabularyAdded: number; vocabularyDeduplicated: number; vocabularyConflicts: string[];
}
export interface BackupReceipt {
  archiveId: string; meetingCount: number; assetCount: number; omittedPcmCount: number;
  omittedLogCount: number; size: number; sha256: string;
}
export interface RestoreReceipt {
  batchId: string; archiveId: string; meetingIds: string[]; alreadyPublished: boolean;
  vocabularyAdded: number | null; vocabularyDeduplicated: number | null; vocabularyConflicts: string[] | null;
}
export interface DataSnapshot {
  revision: number; token: string | null; kind: DataKind | null; phase: string; busy: boolean;
  backupPreview: BackupPreview | null; restorePreview: RestorePreview | null;
  backupReceipt: BackupReceipt | null; restoreReceipt: RestoreReceipt | null;
  recoveryPreview: RecoveryPreview | null; recoveryReceipt: RecoveryReceipt | null;
  relocationPreview: MovePreview | null; relocationReceipt: MoveReceipt | null; restartRequired: boolean;
  deletionPreview?: DeletionPreview | null; deletionReceipt?: DeletionReceipt | null;
  errorMessage: string | null; attentionRequired: boolean; retryAllowed: boolean;
}
export interface DataRequest { token: string; confirmationId: string; action: "refresh" | "publish" | "recover" | "keep" | "delete_including_unregistered" }
export interface DataAvailability { available: boolean; reason: string | null }

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const integer = (n: unknown): n is number => Number.isSafeInteger(n) && (n as number) >= 0;
const texts = (values: unknown): values is string[] => Array.isArray(values) && values.length <= 65536 && values.every(v => typeof v === "string" && !v.includes("\0"));
export function assertDataSnapshot(s: DataSnapshot): void {
  const fail = () => { throw Error("バックアップ／復元の状態が不正です。状態を再確認してください。"); };
  if (!s || !integer(s.revision) || !["idle", "choosing", "verifying", "reading", "reconstructing", "publishing", "copying", "handoff", "switching", "ready", "completed", "failed", "cancelling", "cancelled", "discarding", "attention", "verifying_delete_scope", "fixing_delete_scope", "delete_scope_fixed", "deleting_files", "deleting_database"].includes(s.phase)
    || typeof s.busy !== "boolean" || typeof s.attentionRequired !== "boolean" || typeof s.retryAllowed !== "boolean"
    || typeof s.restartRequired !== "boolean"
    || (s.token !== null && (typeof s.token !== "string" || !uuid.test(s.token))) || ![null, "backup", "restore", "recovery", "relocation", "deletion"].includes(s.kind)
    || (s.errorMessage !== null && typeof s.errorMessage !== "string")) fail();
  if (["verifying_delete_scope", "fixing_delete_scope", "delete_scope_fixed", "deleting_files", "deleting_database"].includes(s.phase) && (s.kind !== "deletion" || !s.busy)) fail();
  if (s.backupPreview !== null) {
    const p = s.backupPreview;
    if (!p || s.kind !== "backup" || !uuid.test(p.confirmationId) || typeof p.destination !== "string" || !p.destination || p.destination.includes("\0") || typeof p.replacesExisting !== "boolean") fail();
  }
  if (s.restorePreview !== null) {
    const p = s.restorePreview;
    if (!p || s.kind !== "restore" || ![p.confirmationId, p.batchId, p.archiveId].every(id => typeof id === "string" && uuid.test(id))
      || !texts(p.meetingTitles) || !integer(p.requiredBytes) || !integer(p.availableBytes) || !integer(p.vocabularyAdded) || !integer(p.vocabularyDeduplicated) || !texts(p.vocabularyConflicts)) fail();
  }
  if (s.backupReceipt !== null) {
    const r = s.backupReceipt;
    if (!r || s.kind !== "backup" || !uuid.test(r.archiveId) || !/^[0-9a-f]{64}$/u.test(r.sha256) || ![r.meetingCount, r.assetCount, r.omittedPcmCount, r.omittedLogCount, r.size].every(integer)) fail();
  }
  if (s.restoreReceipt !== null) {
    const r = s.restoreReceipt;
    if (!r || s.kind !== "restore" || !uuid.test(r.batchId) || !uuid.test(r.archiveId) || !texts(r.meetingIds) || !r.meetingIds.every(id => uuid.test(id))
      || new Set(r.meetingIds).size !== r.meetingIds.length || typeof r.alreadyPublished !== "boolean"
      || (r.vocabularyAdded !== null && !integer(r.vocabularyAdded)) || (r.vocabularyDeduplicated !== null && !integer(r.vocabularyDeduplicated))
      || (r.vocabularyConflicts !== null && !texts(r.vocabularyConflicts))
      || (s.restorePreview && (r.batchId !== s.restorePreview.batchId || r.archiveId !== s.restorePreview.archiveId))) fail();
  }
  if (s.recoveryPreview !== null) { assertRecoveryPreview(s.recoveryPreview); if (s.kind !== "recovery") fail(); }
  if (s.recoveryReceipt !== null) {
    assertRecoveryReceipt(s.recoveryReceipt);
    if (s.kind !== "recovery" || (s.recoveryPreview && (s.recoveryReceipt.operationId !== s.recoveryPreview.item.operationId || s.recoveryReceipt.source !== s.recoveryPreview.item.source))) fail();
  }
  const path = (p: unknown): p is string => typeof p === "string" && p.startsWith("/") && p.length <= 4096 && !p.includes("\0");
  if (s.relocationPreview !== null) {
    const p = s.relocationPreview;
    if (!p || s.kind !== "relocation" || !uuid.test(p.confirmationId) || !uuid.test(p.operationId)
      || !path(p.source) || !path(p.destination) || p.source === p.destination
      || ![p.meetingCount, p.requiredBytes, p.availableBytes].every(integer) || (p.fileCount !== null && !integer(p.fileCount))
      || typeof p.pending !== "boolean" || typeof p.resumeAllowed !== "boolean" || (p.resumeReason !== null && (typeof p.resumeReason !== "string" || !p.resumeReason))
      || (p.resumeAllowed !== (p.resumeReason === null))
      || ((p.copiedPaths === null) !== (p.expectedPaths === null))
      || (p.copiedPaths !== null && (!integer(p.copiedPaths) || !integer(p.expectedPaths) || p.copiedPaths > p.expectedPaths!))
      || (!p.pending && (!p.resumeAllowed || p.fileCount === null || p.copiedPaths !== null))
      || (p.pending && p.resumeAllowed && (p.fileCount === null || p.copiedPaths === null))) fail();
  }
  if (s.relocationReceipt !== null) {
    const r = s.relocationReceipt, p = s.relocationPreview;
    if (!r || s.kind !== "relocation" || !uuid.test(r.operationId) || !path(r.source) || !path(r.destination) || r.source === r.destination
      || typeof r.retained !== "boolean" || r.restartRequired !== !r.retained || s.restartRequired !== r.restartRequired
      || !p || (r.retained && !p.pending) || p.operationId !== r.operationId || p.source !== r.source || p.destination !== r.destination) fail();
  }
  if (s.deletionPreview != null) { assertDeletionPreview(s.deletionPreview); if (s.kind !== "deletion") fail(); }
  if (s.deletionReceipt != null) {
    assertDeletionReceipt(s.deletionReceipt, s.deletionPreview);
    if (s.kind !== "deletion" || s.restartRequired !== s.deletionReceipt.restartRequired) fail();
  }
  const pendingDeletion = s.kind === "deletion" && !!s.deletionPreview?.pending;
  const hasResult = !!(s.backupReceipt || s.restoreReceipt || s.recoveryReceipt || s.relocationReceipt || s.deletionReceipt);
  if ((s.phase === "completed") !== hasResult || (hasResult && (s.busy || s.attentionRequired || s.retryAllowed))
    || (s.phase === "ready" && (s.busy || !s.retryAllowed || !(s.backupPreview || s.restorePreview || s.recoveryPreview || s.relocationPreview || s.deletionPreview)))
    || (s.restartRequired && !pendingDeletion && (s.busy || s.retryAllowed || !["relocation", "deletion"].includes(s.kind!) || (!s.relocationReceipt && !s.deletionReceipt && !s.attentionRequired)))
    || ((s.busy || s.retryAllowed || hasResult || s.attentionRequired) && (!s.token || !s.kind))) fail();
}
export function acceptDataSnapshot(current: DataSnapshot | null, next: DataSnapshot): DataSnapshot {
  assertDataSnapshot(next);
  if (current && current.revision > next.revision) return current;
  if (current && current.revision === next.revision) {
    if (JSON.stringify(current) !== JSON.stringify(next)) throw Error("同じ版のデータ操作状態が一致しません。状態を再確認してください。");
    return current;
  }
  return next;
}
export function dataRequest(s: DataSnapshot, action: DataRequest["action"]): DataRequest {
  assertDataSnapshot(s);
  if (s.kind === "deletion") {
    const p = s.deletionPreview;
    if (!s.token || !p || s.busy || s.deletionReceipt || (s.restartRequired && !p.pending)) throw Error("削除の現在の状態を再確認してください。");
    const resolution = action === "refresh" || (action === "keep" && p.keepAllowed);
    if (s.attentionRequired && !resolution) throw Error("部分完了の状態を再検査してください。");
    if (!s.retryAllowed && !resolution) throw Error("削除確認が失効しています。");
    if (action === "keep" && !p.keepAllowed) throw Error("開始済み削除を保持・中止して復元することはできません。");
    if (action === "recover" || (action === "delete_including_unregistered" && (!p.unregisteredCount || !["review", "not_started"].includes(p.phase)))) throw Error("確認した対象と削除の判断が一致しません。");
    if (["publish", "delete_including_unregistered"].includes(action) && (s.phase !== "ready" || !p.executeAllowed
      || (action === "publish" && p.unregisteredCount > 0 && ["review", "not_started"].includes(p.phase)))) throw Error("対象と未登録物の判断を再確認してください。");
    return { token: s.token, confirmationId: p.confirmationId, action };
  }
  if (action === "delete_including_unregistered") throw Error("削除以外の操作には使用できません。");
  const id = s.backupPreview?.confirmationId ?? s.restorePreview?.confirmationId ?? s.recoveryPreview?.confirmationId ?? s.relocationPreview?.confirmationId;
  if (!s.token || !id || s.busy || !s.retryAllowed || s.attentionRequired || s.restartRequired || (action === "refresh" && s.kind !== "restore" && s.kind !== "recovery" && s.kind !== "relocation")) throw Error("確認済みの操作を再確認してください。");
  const keepingMove = action === "keep" && s.kind === "relocation" && s.relocationPreview?.pending;
  if (!keepingMove && (s.kind === "recovery") !== ["recover", "keep"].includes(action) && action !== "refresh") throw Error("対象と実行する操作が一致しません。");
  if (["recover", "keep"].includes(action) && (s.phase !== "ready" || (!keepingMove && (s.kind !== "recovery" || !s.recoveryPreview)))) throw Error("最新の確認内容を再確認してください。");
  if (action === "publish" && s.kind === "restore" && (s.phase !== "ready" || !s.restorePreview || s.restorePreview.availableBytes < s.restorePreview.requiredBytes)) throw Error("復元内容と容量を再確認してください。");
  if (action === "publish" && s.kind === "relocation" && (s.phase !== "ready" || !s.relocationPreview?.resumeAllowed || s.relocationPreview.availableBytes < s.relocationPreview.requiredBytes)) throw Error("保存先と容量を再確認してください。");
  return { token: s.token, confirmationId: id, action };
}
export const dataPhaseLabel = (phase: string) => ({ idle: "操作を選択", choosing: "ファイル・フォルダー選択中", reading: "バックアップを読み込み中", verifying: "内容と容量を検証中", reconstructing: "追加する会議を検証中", copying: "会議DBと管理音声をコピー中", handoff: "回復履歴と所有台帳を検証中", switching: "保存先を切替中", ready: "確認待ち", publishing: "保存を確定中", completed: "完了", failed: "要対応", cancelling: "安全に取消中", cancelled: "取消済み", discarding: "確認用コピーを整理中", attention: "保存結果の確認が必要" } as Record<string, string>)[phase] ?? phase;
