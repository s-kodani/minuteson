// Only interrupted automatic cleanup remains visible. There is no manual model
// deletion confirmation or publication flow in the initial implementation.
export type ModelDeletionStartupReason = "model_completed_scope_reappeared" | "model_completed_receipt_invalid" | "model_completed_scope_uninspectable";
export interface ModelBodyDeletionItem { operationId: string; state: string; packageId: string | null; modelName: string | null; reviewAllowed: boolean; errorMessage: string | null; startupReason?: ModelDeletionStartupReason | null }
export interface ModelBodyDeletionListing { items: ModelBodyDeletionItem[]; writeBlocked: boolean; restartRequired: boolean; reason: string | null }

export function modelDeletionStartupLabel(reason: ModelDeletionStartupReason): string {
  switch (reason) {
    case "model_completed_scope_reappeared": return "完了済みモデル削除の対象が再出現しています。自動では削除しません。現在の対象を確認し、新しい同意で解決してください。";
    case "model_completed_receipt_invalid": return "完了済みモデル削除の記録・同意履歴を検証できません。対象を保持し、通常書込みを停止しています。";
    case "model_completed_scope_uninspectable": return "完了済みモデル削除の保存領域・対象を安全に検査できません。対象を保持し、通常書込みを停止しています。";
  }
}

const id = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(v);
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0 && !v.includes("\0") && new TextEncoder().encode(v).length <= 4096;
export function assertModelBodyDeletionListing(l: ModelBodyDeletionListing): void {
  const fail = (): never => { throw Error("モデル削除の状態が不正です。対象を再確認してください。"); };
  if (!l || !Array.isArray(l.items) || l.items.length > 100 || typeof l.writeBlocked !== "boolean" || typeof l.restartRequired !== "boolean"
    || (l.reason !== null && !text(l.reason)) || l.writeBlocked !== (l.reason !== null) || (l.restartRequired && !l.writeBlocked)) fail();
  for (const item of l.items) if (!item || !id(item.operationId) || !text(item.state) || typeof item.reviewAllowed !== "boolean"
    || (item.startupReason != null && (!["model_completed_scope_reappeared", "model_completed_receipt_invalid", "model_completed_scope_uninspectable"].includes(item.startupReason) || item.state !== "completed" || !l.writeBlocked))
    || (item.packageId !== null && !id(item.packageId)) || (item.modelName !== null && !text(item.modelName)) || (item.errorMessage !== null && !text(item.errorMessage))
    || (item.reviewAllowed ? item.packageId === null || item.modelName === null || item.errorMessage !== null : item.packageId !== null || item.modelName !== null || item.errorMessage === null)) fail();
  if (new Set(l.items.map(i => i.operationId)).size !== l.items.length) fail();
}
