import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state.ts";
import { assertModelBodyDeletionPage, type ModelBodyDeletionPage, type ModelBodyDeletionPreview } from "./model-deletion-operation-state.ts";

export const modelDeletionNewScope = (p: ModelBodyDeletionPreview): boolean => p.phase === "review" || p.phase === "reconfirmation";
export const modelDeletionPhaseLabel = (phase: string): string => ({ review: "新規の対象確認", not_started: "同意保存済み・本体削除は未開始", files: "本体削除の途中", reconfirmation: "変更後の残り範囲への再同意", awaiting_ack: "本体処理済み・完了確認待ち", retained: "本体削除を開始せず保持", completed: "本体削除完了・履歴は保持" } as Record<string, string>)[phase] ?? phase;

// Consent and the frozen page belong to exactly one server confirmation. Never
// carry them through a refresh, a pending resume or a playback/save wait.
export function modelDeletionPublishRequest(s: DataSnapshot, confirmationId: string, page: ModelBodyDeletionPage | null, understoodId: string | null, includeUnknownId: string | null): DataRequest {
  const p = s.kind === "model_deletion" ? s.modelDeletionPreview : null;
  if (!p || p.confirmationId !== confirmationId || understoodId !== confirmationId || !page) throw Error("モデル削除の新しい一覧と同意を確認してください。");
  assertModelBodyDeletionPage(page, p, page.offset);
  const unknown = modelDeletionNewScope(p) && p.unregisteredCount > 0;
  if (unknown && includeUnknownId !== confirmationId) throw Error("未登録ファイルには別の同意が必要です。");
  return dataRequest(s, unknown ? "delete_including_unregistered" : "publish");
}

// Native removal walks the frozen sorted plan backwards. A processed path may
// be a directory or an already-missing entry, not necessarily a removed file.
export function modelDeletionPathProcessed(p: ModelBodyDeletionPreview, index: number): boolean {
  return index >= 0 && index < p.pathCount && index >= p.pathCount - p.processedPaths;
}
