import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state.ts";
import { assertAudioDeletionPage, type AudioDeletionPage, type AudioDeletionPreview } from "./audio-deletion-operation-state.ts";

export const audioDeletionNewScope = (p: AudioDeletionPreview): boolean => p.phase === "review" || p.phase === "reconfirmation";
export const audioDeletionPhaseLabel = (phase: string): string => ({ review: "新規の対象確認", not_started: "同意保存済み・音声削除は未開始", files: "音声削除の途中", availability: "音声の利用不可を確定中", awaiting_ack: "音声処理済み・完了確認待ち", reconfirmation: "再出現した音声への新しい同意", retained: "音声削除を開始せず保持", completed: "音声削除完了・文書と履歴は保持", verifying_audio_delete_scope: "音声の対象を検査中", fixing_audio_delete_scope: "音声削除の同意を保存中", audio_delete_scope_fixed: "同意保存済み", deleting_audio_files: "音声ファイルを削除中", fixing_audio_unavailable: "音声の利用不可を確定中", audio_delete_completed: "完了結果を確定中", cancelling: "安全な停止を待機中" } as Record<string, string>)[phase] ?? phase;
export const audioDeletionDispositionLabel = (disposition: string): string => ({ registered_audio: "登録済み音声", producer_audio_leftover: "録音・取り込み由来の未登録音声", analysis_pcm: "解析用PCM", historical_audio: "元の削除履歴と一致する音声", retain: "保持するファイル", unclassified: "分類できないため保持" } as Record<string, string>)[disposition] ?? disposition;
export const audioDeletionStartupLabel = (reason: string): string => ({ audio_completed_scope_reappeared: "完了済み音声が再出現しました。現在の対象への別同意が必要です。", audio_completed_receipt_invalid: "音声削除の完了記録を検証できません。自動削除せず保持しています。", audio_completed_scope_uninspectable: "完了済み音声の範囲を検査できません。対象を保持しています。", restored_audio_scope_reappeared: "復元された音声が再出現しました。復元履歴の解決は未対応のため保持しています。", restored_audio_receipt_invalid: "復元された音声削除の記録を検証できません。対象を保持しています。", restored_audio_scope_uninspectable: "復元された音声の範囲を検査できません。対象を保持しています。" } as Record<string, string>)[reason] ?? reason;

// A displayed page is a frozen observation, not permission to delete its paths.
// Both checkboxes belong to this confirmation; only native IDs leave the UI.
export function audioDeletionPublishRequest(s: DataSnapshot, confirmationId: string, page: AudioDeletionPage | null, understoodId: string | null, includeUnknownId: string | null): DataRequest {
  const p = s.kind === "audio_deletion" ? s.audioDeletionPreview : null;
  if (!p || p.confirmationId !== confirmationId || understoodId !== confirmationId || !page) throw Error("音声削除の新しい一覧と同意を確認してください。");
  assertAudioDeletionPage(page, p, page.offset);
  const unknown = audioDeletionNewScope(p) && p.unregisteredAudioFiles > 0;
  if (unknown && includeUnknownId !== confirmationId) throw Error("未登録音声には別の同意が必要です。分類できないファイルは保持します。");
  return dataRequest(s, unknown ? "delete_including_unregistered" : "publish");
}
