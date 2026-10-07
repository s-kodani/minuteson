export type RecoverySource = "scratch" | "restore_addition";
export interface RecoveryItem { source: RecoverySource; operationId: string; paths: string[]; registeredPaths: number; reason: string; retained: boolean }
export interface RecoveryListing { supported: boolean; root: string; items: RecoveryItem[] }
export interface RecoveryPreview { confirmationId: string; item: RecoveryItem; root: string; observedPaths: number; inspectionComplete: boolean }
export interface RecoveryReceipt { source: RecoverySource; operationId: string; outcome: "recovered" | "retained" | "attention" | "published"; reason: string | null }
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const text = (v: unknown, limit = 8192): v is string => typeof v === "string" && v.length <= limit && !v.includes("\0");
const integer = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0 && (v as number) <= 200000;
const source = (v: unknown) => v === "scratch" || v === "restore_addition";
const root = (v: unknown): v is string => text(v, 4096) && (/^\//u.test(v) || /^[A-Za-z]:[\\/]/u.test(v));
export function assertRecoveryItem(item: RecoveryItem): void {
  if (!item || !source(item.source) || !uuid.test(item.operationId) || typeof item.retained !== "boolean" || !integer(item.registeredPaths) || !text(item.reason)
    || !Array.isArray(item.paths) || item.paths.length > 200000 || new Set(item.paths).size !== item.paths.length
    || !item.paths.every(p => text(p, 1024) && (item.source === "scratch" ? p === `.mon-stage-${item.operationId}` : p.startsWith("meetings/") && uuid.test(p.slice(9))))
    || (item.source === "scratch" && item.paths.length !== 1)) throw Error("要確認対象の記録が不正です。ファイルは変更しません。");
}
export function assertRecoveryListing(list: RecoveryListing): void {
  if (!list || typeof list.supported !== "boolean" || !root(list.root) || !Array.isArray(list.items) || list.items.length > 2000 || (!list.supported && list.items.length > 0)) throw Error("要確認一覧を検証できません。");
  list.items.forEach(assertRecoveryItem);
  if (new Set(list.items.map(i => `${i.source}:${i.operationId}`)).size !== list.items.length) throw Error("要確認一覧の対象が重複しています。");
}
export function assertRecoveryPreview(p: RecoveryPreview): void {
  if (!p || !uuid.test(p.confirmationId) || !root(p.root) || !integer(p.observedPaths) || typeof p.inspectionComplete !== "boolean") throw Error("回収の確認内容が不正です。");
  assertRecoveryItem(p.item);
  if (p.item.retained) throw Error("保持して確認済みの対象は実行しません。");
}
export function assertRecoveryReceipt(r: RecoveryReceipt): void {
  if (!r || !source(r.source) || !uuid.test(r.operationId) || !["recovered", "retained", "attention", "published"].includes(r.outcome) || (r.reason !== null && !text(r.reason)) || (r.outcome === "attention" && !r.reason)) throw Error("回収の結果を確認できません。");
}
export const recoverySourceLabel = (s: RecoverySource) => s === "scratch" ? "バックアップ・復元の構築用コピー" : "未公開の追加復元データ";
export const recoveryOutcomeLabel = (r: RecoveryReceipt) => ({ recovered: "記録済みの対象を回収しました。", retained: "ファイルを保持して確認済みと記録しました。回収はしていません。", attention: "要確認が残っています。所有不明・変更済みのファイルは保持しています。", published: "公開済みの会議は保持しました。" })[r.outcome];
export const recoveryOutcomeStatus = (r: RecoveryReceipt) => ({ recovered: "回収済み", retained: "保持して確認済み（回収なし）", attention: "要確認が残っています", published: "公開済み会議を保持" })[r.outcome];
