import { assertRecoveryPreview, assertRecoveryReceipt, type RecoveryPreview, type RecoveryReceipt } from "./recovery-state.ts";
export type DataKind = "backup" | "restore" | "recovery";
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
  errorMessage: string | null; attentionRequired: boolean; retryAllowed: boolean;
}
export interface DataRequest { token: string; confirmationId: string; action: "refresh" | "publish" | "recover" | "keep" }
export interface DataAvailability { available: boolean; reason: string | null }

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const integer = (n: unknown): n is number => Number.isSafeInteger(n) && (n as number) >= 0;
const texts = (values: unknown): values is string[] => Array.isArray(values) && values.length <= 65536 && values.every(v => typeof v === "string" && !v.includes("\0"));
export function assertDataSnapshot(s: DataSnapshot): void {
  const fail = () => { throw Error("バックアップ／復元の状態が不正です。状態を再確認してください。"); };
  if (!s || !integer(s.revision) || !["idle", "choosing", "verifying", "reading", "reconstructing", "publishing", "ready", "completed", "failed", "cancelling", "cancelled", "discarding", "attention"].includes(s.phase)
    || typeof s.busy !== "boolean" || typeof s.attentionRequired !== "boolean" || typeof s.retryAllowed !== "boolean"
    || (s.token !== null && (typeof s.token !== "string" || !uuid.test(s.token))) || ![null, "backup", "restore", "recovery"].includes(s.kind)
    || (s.errorMessage !== null && typeof s.errorMessage !== "string")) fail();
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
  const hasResult = !!(s.backupReceipt || s.restoreReceipt || s.recoveryReceipt);
  if ((s.phase === "completed") !== hasResult || (hasResult && (s.busy || s.attentionRequired || s.retryAllowed))
    || (s.phase === "ready" && (s.busy || !s.retryAllowed || !(s.backupPreview || s.restorePreview || s.recoveryPreview)))
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
  const id = s.backupPreview?.confirmationId ?? s.restorePreview?.confirmationId ?? s.recoveryPreview?.confirmationId;
  if (!s.token || !id || s.busy || !s.retryAllowed || s.attentionRequired || (action === "refresh" && s.kind !== "restore" && s.kind !== "recovery")) throw Error("確認済みの操作を再確認してください。");
  if ((s.kind === "recovery") !== ["recover", "keep"].includes(action) && action !== "refresh") throw Error("対象と実行する操作が一致しません。");
  if (["recover", "keep"].includes(action) && (s.kind !== "recovery" || s.phase !== "ready" || !s.recoveryPreview)) throw Error("最新の回収内容を再確認してください。");
  if (action === "publish" && s.kind === "restore" && (s.phase !== "ready" || !s.restorePreview || s.restorePreview.availableBytes < s.restorePreview.requiredBytes)) throw Error("復元内容と容量を再確認してください。");
  return { token: s.token, confirmationId: id, action };
}
export const dataPhaseLabel = (phase: string) => ({ idle: "操作を選択", choosing: "ファイル選択中", reading: "バックアップを読み込み中", verifying: "内容と容量を検証中", reconstructing: "追加する会議を検証中", ready: "確認待ち", publishing: "保存を確定中", completed: "完了", failed: "要対応", cancelling: "安全に取消中", cancelled: "取消済み", discarding: "確認用コピーを整理中", attention: "保存結果の確認が必要" } as Record<string, string>)[phase] ?? phase;
