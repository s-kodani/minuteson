import type {ArchivedInputView} from "./archived-input-controller.ts";

/** Explain what could not be checked, without claiming a missing audio range. */
export function archivedDiagnosticMessage(view:ArchivedInputView):string {
  const details=[
    view.unavailable?"過去の接続状況を確認できません。接続記録や通知を読み取れなかったため表示しています。自動で再確認します。":"",
    view.historyIssue==="missing"?"録音中の接続記録が見つからず、途中で音が途切れたかを確認できません。":view.historyIssue==="invalid"?"録音中の接続記録を正しく読み取れず、途中で音が途切れたかを確認できません。":"",
    view.deliveryUnavailable?"復元した会議の通知を表示する準備ができていません。":"",
    view.historySaveIssue?({failed:"接続記録の保存処理でエラーが発生しました。",invalid:"接続記録に不整合が見つかり、記録の保存を停止しました。",unconfirmed:"録音終了時に接続記録の保存完了を確認できませんでした。",unavailable:"録音終了時の接続記録の保存結果を読み取れません。"}[view.historySaveIssue]):"",
    view.loss.microphone>0?`マイクの接続通知を${view.loss.microphone}件残せませんでした。すべての中断・復帰を記録できていないため、接続状況を確認できません。`:"",
    view.loss.system>0?`PC音声の接続通知を${view.loss.system}件残せませんでした。すべての中断・復帰を記録できていないため、接続状況を確認できません。`:"",
    ["microphone","system"].some(input=>view.reception[input as "microphone"|"system"]==="unconfirmed")?"通知を確認した結果の反映を待っています。":""
  ].filter(Boolean);
  if(!details.length)return "";
  const checkAudio=view.historyIssue!==null||view.historySaveIssue!==null||view.loss.microphone>0||view.loss.system>0;
  return `終了した録音「${view.title}」：${details.join(" ")} この通知だけで録音ファイルの保存失敗や音声の欠落が分かったわけではありません。${checkAudio?"会議を開き、音声を再生して内容を確認してください。":"確認でき次第、通知一覧を解消済みに更新します。"}`;
}
