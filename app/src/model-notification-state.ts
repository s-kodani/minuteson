import type { ToastLevel } from "./toast-state.ts";

export interface ModelNotice {
  outcome: ModelOutcome;
  eventId: string;
  message: string;
  level: ToastLevel;
  details: string;
}
export type ModelOutcome = "inspected" | "placed" | "cpuSucceeded" | "cpuCancelled" | "applied" | "applicationStopped" | "applicationReleased";

/** Call only after the originating controller validates its exact receipt.
 * A stop request is not proof that an application was rolled back. */
export function modelNotice(outcome: ModelOutcome, eventId: string, subject = "モデル"): ModelNotice {
  if (!eventId) throw Error("モデル通知には操作IDが必要です。");
  const outcomes: Record<ModelOutcome, Omit<ModelNotice, "eventId" | "outcome">> = {
    inspected: { message: "モデルの事前検査が完了しました", level: "info", details: "ファイルの検査記録です。配布元の真正性・ライセンス・推論互換性は未検証で、モデルは未配置です。実行ファイルは起動していません。" },
    placed: { message: "モデルを管理領域へコピーしました", level: "success", details: "管理領域への配置を記録しました。エンジン互換性は未検証で、AI準備完了ではありません。元ファイル・既存ジョブ・設定は変更していません。" },
    cpuSucceeded: { message: "合成入力でCPU互換性を確認しました", level: "success", details: "固定した合成入力だけのCPU確認です。プロファイルは変更していません。精度・速度・実データ・通常条件での互換性は未評価です。適用状態は構成記録を確認してください。" },
    cpuCancelled: { message: "CPU互換性確認を取り消しました", level: "info", details: "処理終了を確認し、ログを保全しました。互換性確認の成功ではありません。自動再実行はしません。" },
    applied: { message: `${subject}のモデル選択を保存しました`, level: "success", details: "既存ジョブは変更していません。次の新規解析から設定を使用します。解析の開始・完了やAI準備完了を示す通知ではありません。品質・速度・通常条件での互換性は未評価です。" },
    applicationStopped: { message: "適用処理が終了しました。保存結果の確認が必要です", level: "warning", details: "取消要求後の処理が終了しました。保存が先に確定した可能性があります。同じ操作の結果と構成記録を確認してください。取消要求だけでは未保存と判断できません。" },
    applicationReleased: { message: "未確定の適用結果の確認を終了しました", level: "info", details: "利用者の確認により、同じ操作の照会画面を終了しました。保存成功・取消・巻き戻しを確定する操作ではありません。現在の構成記録は保持し、新しい適用には現在の条件で別の確認が必要です。" },
  };
  return { outcome, eventId, ...outcomes[outcome] };
}
