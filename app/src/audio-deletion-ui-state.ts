import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state.ts";
import { assertAudioDeletionPage, type AudioDeletionPage, type AudioDeletionPreview } from "./audio-deletion-operation-state.ts";

export const audioDeletionNewScope = (p: AudioDeletionPreview): boolean => p.phase === "review" || p.phase === "reconfirmation";
export const audioDeletionSimpleReview = (p: AudioDeletionPreview): boolean => p.phase === "review" && !p.pending;

// The two-button confirmation authorizes all classified audio in this native
// review, including unregistered audio. Recovery retains its recorded policy.
export function audioDeletionSimplePublishRequest(s: DataSnapshot, confirmationId: string): DataRequest {
  const p = s.kind === "audio_deletion" ? s.audioDeletionPreview : null;
  if (!p || p.confirmationId !== confirmationId || !audioDeletionSimpleReview(p)) throw Error("音声削除の新しい確認を開いてください。");
  return dataRequest(s, p.unregisteredAudioFiles > 0 ? "delete_including_unregistered" : "publish");
}
export const audioDeletionPhaseLabel = (phase: string): string => ({ review: "新規の対象確認", not_started: "削除要求を受付済み・音声削除は未開始", files: "音声削除の途中", availability: "音声の利用不可を確定中", awaiting_ack: "音声処理済み・完了確認待ち", restored_unavailable: "復元元の音声利用不可記録・削除要求なし", reconfirmation: "残りの音声の確認待ち", retained: "音声削除を開始せず保持", completed: "音声削除完了・文書と履歴は保持", verifying_audio_delete_scope: "音声の対象を検査中", fixing_audio_delete_scope: "音声削除の要求を受け付けています", audio_delete_scope_fixed: "削除要求を受付済み", deleting_audio_files: "音声ファイルを削除中", fixing_audio_unavailable: "音声の利用不可を確定中", audio_delete_completed: "完了結果を確定中", cancelling: "安全な停止を待機中" } as Record<string, string>)[phase] ?? phase;
export const audioDeletionDispositionLabel = (disposition: string): string => ({ registered_audio: "登録済み音声", producer_audio_leftover: "録音・取り込み由来の未登録音声", analysis_pcm: "解析用PCM", historical_audio: "元の削除履歴と一致する音声", retain: "保持するファイル", unclassified: "分類できないため保持" } as Record<string, string>)[disposition] ?? disposition;
export function audioDeletionPublishRequest(s: DataSnapshot, confirmationId: string, page: AudioDeletionPage | null, understoodId: string | null, includeUnknownId: string | null): DataRequest {
  const p = s.kind === "audio_deletion" ? s.audioDeletionPreview : null;
  if (!p || p.confirmationId !== confirmationId || understoodId !== confirmationId || !page) throw Error("音声削除の新しい一覧と削除要求を確認してください。");
  assertAudioDeletionPage(page, p, page.offset);
  const unknown = audioDeletionNewScope(p) && p.unregisteredAudioFiles > 0;
  if (unknown && includeUnknownId !== confirmationId) throw Error("未登録音声には別の削除要求が必要です。分類できないファイルは保持します。");
  return dataRequest(s, unknown ? "delete_including_unregistered" : "publish");
}
