import type { DataOperation } from "./data-operation";
import type { DataAvailability } from "./data-operation-state";

/** Older native builds only report the general data-operation admission. */
export function deletionAvailability(value: DataAvailability | null) {
  return value?.deletionAvailable === undefined
    ? { available: !!value?.available, reason: value?.reason ?? null, ...(value?.reasonCode !== undefined ? { reasonCode: value.reasonCode } : {}) }
    : { available: value.deletionAvailable, reason: value.deletionReason ?? null, ...(value.deletionReasonCode !== undefined ? { reasonCode: value.deletionReasonCode } : {}) };
}

/** Only an explicit native restart code can admit the restricted recovery read.
 * The translated explanation is display text and never authorizes recovery. */
export function permitsDeletionRecovery(value: Pick<DataAvailability, "reasonCode"> | null, restartRequired: boolean | undefined): boolean {
  return !!restartRequired && value?.reasonCode === "relocation_restart_required";
}

/** Explain the same gates used by deletion admission; never authorize a write. */
export function deletionReviewReason(operation: DataOperation, state: {
  native: boolean; supported: boolean; locked: boolean; protectedNow: boolean; sharedAvailable: boolean; availabilityReason?: string | null;
}): string {
  if (!state.native) return "削除はデスクトップアプリで利用できます。";
  if (!operation.connected) return operation.error ? `削除の状態を確認できません：${operation.error}` : "削除の状態を確認しています。";
  if (!state.supported) return "このアプリでは削除機能を利用できません。更新後にお試しください。";
  if (operation.snapshot?.restartRequired) return "保存先を切り替えたため、アプリの再起動後に削除できます。";
  if (operation.deletionListing?.writeBlocked || operation.audioDeletionListing?.writeBlocked || operation.modelDeletionListing?.writeBlocked
    || operation.snapshot?.deletionPreview?.pending || operation.snapshot?.audioDeletionPreview?.pending || operation.snapshot?.modelDeletionPreview?.pending) return "未完了の削除があります。設定画面で確認してください。";
  if (state.protectedNow) return "前の削除操作の完了・結果確認をお待ちください。";
  if (operation.snapshot?.busy) return "データ操作が完了すると削除できます。";
  const reason = state.availabilityReason === undefined ? operation.availability?.reason : state.availabilityReason;
  if (!state.sharedAvailable) return reason ? `${reason}のため削除できません。` : "削除できる状態か確認しています。";
  if (state.locked) return "ほかの操作や録音テストが完了すると削除できます。";
  return "削除の準備が完了するまでお待ちください。";
}
