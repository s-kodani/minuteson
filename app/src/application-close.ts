import { assertDataSnapshot, type DataSnapshot } from "./data-operation-state.ts";
import type { ImportSnapshot } from "./import-state.ts";
import type { RecordingSnapshot } from "./session.ts";

export interface CloseActivity { analysisBusy: boolean; modelPreparationBusy: boolean; recordingRecoveryBusy?: boolean }
export interface CloseContext { recording: RecordingSnapshot; importing: ImportSnapshot; data: DataSnapshot; activity: CloseActivity }
const text = (v: unknown) => v === null || typeof v === "string" && !!v && !v.includes("\0");
const count = (v: unknown) => Number.isSafeInteger(v) && (v as number) >= 0;
export const recordingActive = (s: RecordingSnapshot) => ["preparing", "recording", "stopping", "saving"].includes(s.phase);
export const importNeedsConsent = (s: ImportSnapshot) => s.busy || s.phase === "ready";
export const dataNeedsConsent = (s: DataSnapshot) => s.busy || s.retryAllowed || !!(s.deletionPreview?.pending || s.audioDeletionPreview?.pending);
export function assertCloseRecording(s: RecordingSnapshot) {
  if (!s || !count(s.revision) || !count(s.elapsedMs) || !["idle", "preparing", "recording", "stopping", "saving", "error"].includes(s.phase) || !text(s.sessionId) || !text(s.meetingId) || !text(s.errorMessage) || typeof s.microphoneConnected !== "boolean" || typeof s.systemAudioConnected !== "boolean" || (recordingActive(s) && (!s.sessionId || !s.meetingId)) || s.phase === "idle" && (s.sessionId !== null || s.meetingId !== null)) throw Error("録音状態を検証できません。停止・終了は開始していません。");
}
export function assertCloseImport(s: ImportSnapshot) {
  if (!s || !count(s.revision) || !["idle", "choosing", "inspecting", "ready", "waiting", "copying", "decoding", "converting", "saving", "saved", "cancelling", "cancelled", "failed", "recovering"].includes(s.phase) || typeof s.busy !== "boolean" || typeof s.recordingBlocked !== "boolean" || !text(s.token) || !text(s.fileName) || !text(s.errorMessage) || (importNeedsConsent(s) && !s.token)) throw Error("取り込み状態を検証できません。取消・終了は開始していません。");
}
export function assertCloseContext(c: CloseContext) {
  if (c?.activity?.recordingRecoveryBusy !== undefined && typeof c.activity.recordingRecoveryBusy !== "boolean") throw Error("録音回復の終了状態を確認できません。");
  assertCloseRecording(c.recording); assertCloseImport(c.importing); assertDataSnapshot(c.data);
  if (!c.activity || typeof c.activity.analysisBusy !== "boolean" || typeof c.activity.modelPreparationBusy !== "boolean" || (dataNeedsConsent(c.data) && (!c.data.token || !c.data.kind))) throw Error("終了対象を検証できません。最新の状態を再確認してください。");
}
const dataReview = (s: DataSnapshot) => JSON.stringify([s.backupPreview?.confirmationId, s.restorePreview?.confirmationId, s.relocationPreview?.confirmationId, s.deletionPreview?.confirmationId, s.audioDeletionPreview?.confirmationId]);
// Progress and a winning publication are not a different operation. A new
// token, new scope, or an unreviewed admission always requires new consent.
export function assertCloseTargets(expected: CloseContext, actual: CloseContext) {
  assertCloseContext(expected); assertCloseContext(actual);
  const a = expected.importing, b = actual.importing, d = expected.data, e = actual.data;
  if (actual.activity.recordingRecoveryBusy && !expected.activity.recordingRecoveryBusy) throw Error("録音の回復が始まりました。終了内容を再確認してください。");
  if (recordingActive(actual.recording) || actual.recording.phase !== expected.recording.phase || actual.recording.sessionId !== expected.recording.sessionId || actual.recording.meetingId !== expected.recording.meetingId || actual.importing.recordingBlocked || a.token !== b.token || d.token !== e.token || d.kind !== e.kind || !a.busy && (b.busy || a.revision !== b.revision) || !d.busy && (e.busy || d.revision !== e.revision) || dataReview(d) !== dataReview(e) && !(d.busy && !e.busy && e.phase === "completed")) throw Error("終了対象や確認内容が変更されました。この確認を閉じ、最新の内容を再確認してください。取消・終了は開始していません。");
}
export function closeMessage(c: CloseContext) {
  const parts: string[] = [];
  if (importNeedsConsent(c.importing)) parts.push(c.importing.busy ? "進行中の音声取り込みを取り消し、安全な停止を待ちます。未確定の取り込み物だけを整理し、元ファイルと保存済み会議は保持します。" : "音声取り込みの確認内容を破棄します。元ファイルは変更しません。");
  if (dataNeedsConsent(c.data)) {
    const s = c.data;
    parts.push(s.kind === "audio_deletion" ? "音声削除の停止を待ちます。未完了記録・残りの音声・文字起こし・議事録を保持します。既に消した音声は復元しません。" : s.kind === "deletion" ? "会議削除の停止を待ちます。未完了記録と残りの対象を保持します。既に消したデータは復元しません。" : s.kind === "relocation" ? "保存先変更の停止を待ちます。旧保存先と途中コピーを保持し、切替が完了済みなら手動再起動が必要です。" : s.busy ? "バックアップ／追加復元を取り消し、安全な停止を待ちます。公開済みの結果と既存会議は保持します。" : "バックアップ／追加復元の確認内容を破棄します。既存会議と元バックアップは保持します。");
  }
  parts.push("実行中の解析・モデル準備があれば、安全な区切りで停止して終了します。保存済みの音声・文書と確定した途中結果を保持し、解析は再起動後に再実行できます。");
  parts.push("未保存編集を保存し、実際の停止を確認してからアプリを終了します。受付の応答が失われても取消・終了を自動再送しません。");
  return parts.join("\n");
}
export function closeTarget(c: CloseContext, productName = "MinutesOn") {
  const s = c.data;
  return [importNeedsConsent(c.importing) ? c.importing.fileName ?? "選択中の音声取り込み" : "", dataNeedsConsent(s) ? s.backupPreview?.destination ?? s.relocationPreview?.destination ?? s.deletionPreview?.title ?? s.audioDeletionPreview?.meetingId ?? s.restorePreview?.meetingTitles.join("、") ?? "確認中のデータ操作" : "", c.activity.recordingRecoveryBusy ? "録音の回復" : c.activity.analysisBusy || c.activity.modelPreparationBusy ? "実行中の解析・モデル準備" : ""].filter(Boolean).join(" / ") || productName;
}
export async function waitForImportStop(token: string, read: () => Promise<ImportSnapshot>, wait: () => Promise<void> = () => new Promise(resolve => setTimeout(resolve, 200)), now: () => number = Date.now) {
  const deadline = now() + 120_000;
  for (;;) {
    const s = await read(); assertCloseImport(s);
    if (s.token !== token) throw Error("取り込みの停止対象が変更されました。取消は再送していません。");
    if (!s.busy) return s;
    if (now() >= deadline) throw Error("取り込みの安全な停止がまだ完了していません。取消は再送していません。");
    await wait();
  }
}
export async function waitForRecordingSave(expected: RecordingSnapshot, read: () => Promise<RecordingSnapshot>, wait: () => Promise<void> = () => new Promise(resolve => setTimeout(resolve, 200)), now: () => number = Date.now) {
  assertCloseRecording(expected);
  const deadline = now() + 120_000;
  for (;;) {
    const s = await read(); assertCloseRecording(s);
    if (s.revision < expected.revision) throw Error("古い録音状態では保存完了を確認できません。");
    if (s.phase === "idle") return;
    if (s.sessionId !== expected.sessionId || s.meetingId !== expected.meetingId) throw Error("録音の保存対象が変更されました。停止・終了は再送していません。");
    if (s.phase === "error") throw Error(s.errorMessage ?? "録音の保存に失敗しました。");
    if (now() >= deadline) throw Error("録音の保存がまだ完了していません。停止・終了は再送していません。");
    await wait();
  }
}
