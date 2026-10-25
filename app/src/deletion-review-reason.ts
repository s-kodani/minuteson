import type { DataOperation } from "./data-operation";

/** Explain the same gates used by deletion admission; never authorize a write. */
export function deletionReviewReason(operation: DataOperation, state: {
  native: boolean; supported: boolean; locked: boolean; protectedNow: boolean; sharedAvailable: boolean;
}): string {
  if (!state.native) return "削除はデスクトップアプリで利用できます。";
  if (!operation.connected) return operation.error ? `削除の状態を確認できません：${operation.error}` : "削除の状態を確認しています。";
  if (!state.supported) return "このアプリでは削除機能を利用できません。更新後にお試しください。";
  if (operation.snapshot?.restartRequired) return "保存先を切り替えたため、アプリの再起動後に削除できます。";
  if (operation.deletionListing?.writeBlocked || operation.audioDeletionListing?.writeBlocked || operation.modelDeletionListing?.writeBlocked
    || operation.snapshot?.deletionPreview?.pending || operation.snapshot?.audioDeletionPreview?.pending || operation.snapshot?.modelDeletionPreview?.pending) return "未完了の削除があります。設定画面で確認してください。";
  if (state.protectedNow) return "前の削除操作の完了・結果確認をお待ちください。";
  if (operation.snapshot?.busy) return "データ操作が完了すると削除できます。";
  if (!state.sharedAvailable) return operation.availability?.reason ? `${operation.availability.reason}のため削除できません。` : "削除できる状態か確認しています。";
  if (state.locked) return "ほかの操作や録音テストが完了すると削除できます。";
  return "削除の準備が完了するまでお待ちください。";
}
