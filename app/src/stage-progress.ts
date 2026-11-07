import { bytesLabel } from "./import-state.ts";

export interface StageProgress { stage: string; completed: number; total: number | null; unit: "bytes" | "frames" | "files" | "paths" | "meetings" }
export function checkedStageProgress(value: unknown): StageProgress | null {
  if (value == null) return null;
  const p = value as StageProgress;
  if (!p || typeof p.stage !== "string" || !p.stage || !["bytes", "frames", "files", "paths", "meetings"].includes(p.unit)
    || !Number.isSafeInteger(p.completed) || p.completed < 0
    || !(p.total === null || Number.isSafeInteger(p.total) && p.total >= p.completed)) throw Error("処理工程の進捗を確認できません。");
  return p;
}
export function stageLabel(p: StageProgress | null | undefined): string | undefined {
  if (!p) return undefined;
  return ({publishing_restore:"復元する会議を追加中",copying_backup:"バックアップ音声をコピー中",copying_restore:"復元する音声をコピー中",checking_source:"保存元を再確認中",checking_restore:"復元する内容を検証中",collecting_meetings:"現在の会議内容を収集中",building_meetings:"会議・用語・声の対応を構築中",writing_backup:"バックアップを書き込み中",reading_backup:"バックアップを読み込み中",verifying_backup:"バックアップを検証中",waiting:"使用中の処理・モデルの解放を待機中",preparing:"処理を準備中",checking_audio:"原音声を確認中",
    converting_audio:"音声を変換中",mixing_audio:"会議音声を生成中",verifying_audio:"生成音声を検証中",committing_audio:"音声の保存結果を登録中",
    import_checking_audio:"取り込み音声を検証中",import_creating_wav:"保存用WAVを作成中",import_verifying_audio:"保存内容を検証中",
    import_committing_audio:"保存結果を登録中",import_cleaning_intermediates:"中間ファイルを整理中",
    verifying_model_delete_scope:"モデルの削除対象を確認中",fixing_model_delete_scope:"モデル削除の要求を受け付けています",
    model_delete_scope_fixed:"モデルの削除を準備中",deleting_model_files:"モデルを削除中",
    deleting_files:"確認した項目を削除中",verifying_audio_delete_scope:"音声の削除対象を確認中",
    verifying_delete_scope:"会議の削除対象を確認中",fixing_delete_scope:"会議削除の要求を受け付けています",
    delete_scope_fixed:"会議の削除を準備中",deleting_database:"会議の文書データを削除中",
    fixing_audio_delete_scope:"音声削除の要求を受け付けています",audio_delete_scope_fixed:"音声の削除を準備中",
    deleting_audio_files:"確認した音声を削除中",fixing_audio_unavailable:"音声の削除結果を登録中",
    audio_delete_completed:"音声の削除完了を確認中",model_delete_completed:"モデルの削除完了を確認中"} as Record<string,string>)[p.stage] ?? "確認した処理を実行中";
}
export function stageDetail(value: StageProgress | null | undefined): string | undefined {
  const p = checkedStageProgress(value);
  if (!p || p.completed === 0 && p.total === null) return undefined;
  const count = p.unit === "bytes" ? bytesLabel(p.completed) : p.completed.toLocaleString("ja-JP");
  const total = p.total === null ? "" : ` / ${p.unit === "bytes" ? bytesLabel(p.total) : p.total.toLocaleString("ja-JP")}`;
  return `${p.unit === "bytes" ? "検査の読取り " : ""}${count}${total}${p.unit === "meetings" ? " 会議" : p.unit === "frames" ? " フレーム" : ["paths","files"].includes(p.unit) ? " 項目" : ""}`;
}
