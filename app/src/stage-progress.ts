import { bytesLabel } from "./import-state.ts";

export interface StageProgress { stage: string; completed: number; total: number | null; unit: "bytes" | "frames" | "files" | "paths" }
export function checkedStageProgress(value: unknown): StageProgress | null {
  if (value == null) return null;
  const p = value as StageProgress;
  if (!p || typeof p.stage !== "string" || !p.stage || !["bytes", "frames", "files", "paths"].includes(p.unit)
    || !Number.isSafeInteger(p.completed) || p.completed < 0
    || !(p.total === null || Number.isSafeInteger(p.total) && p.total >= p.completed)) throw Error("処理工程の進捗を確認できません。");
  return p;
}
export function stageLabel(p: StageProgress | null | undefined): string | undefined {
  if (!p) return undefined;
  return ({waiting:"使用中の処理・モデルの解放を待機中",preparing:"処理を準備中",checking_audio:"原音声を確認中",
    converting_audio:"音声を変換中",verifying_audio:"生成音声を検証中",committing_audio:"音声の保存結果を登録中",
    verifying_model_delete_scope:"モデルの削除対象を確認中",fixing_model_delete_scope:"モデル削除の同意を記録中",
    model_delete_scope_fixed:"モデルの削除を準備中",deleting_model_files:"モデルの内容を検査・削除中",
    deleting_files:"確認した項目を削除中",verifying_audio_delete_scope:"音声の削除対象を確認中",
    verifying_delete_scope:"会議の削除対象を確認中"} as Record<string,string>)[p.stage] ?? "確認した処理を実行中";
}
export function stageDetail(value: StageProgress | null | undefined): string | undefined {
  const p = checkedStageProgress(value);
  if (!p || p.completed === 0 && p.total === null) return undefined;
  const count = p.unit === "bytes" ? bytesLabel(p.completed) : p.completed.toLocaleString("ja-JP");
  const total = p.total === null ? "" : ` / ${p.unit === "bytes" ? bytesLabel(p.total) : p.total.toLocaleString("ja-JP")}`;
  return `${p.unit === "bytes" ? "検査の読取り " : ""}${count}${total}${p.unit === "frames" ? " フレーム" : ["paths","files"].includes(p.unit) ? " 項目" : ""}`;
}
