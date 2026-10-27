import type {ArchivedInputView} from "./archived-input-controller.ts";

/** Explain what could not be checked, without claiming a missing audio range. */
export function archivedDiagnosticMessage(view:ArchivedInputView):string {
  const details=[
    view.historyIssue==="missing"?"録音中の接続記録が見つからず、途中で音が途切れたかを確認できません。":view.historyIssue==="invalid"?"録音中の接続記録を正しく読み取れず、途中で音が途切れたかを確認できません。":"",
    view.historySaveIssue==="failed"?"接続記録の保存処理でエラーが発生しました。":view.historySaveIssue==="invalid"?"接続記録に不整合が見つかり、記録の保存を停止しました。":"",
    view.loss.microphone>0?`マイクの接続通知を${view.loss.microphone}件残せませんでした。すべての中断・復帰を記録できていないため、接続状況を確認できません。`:"",
    view.loss.system>0?`PC音声の接続通知を${view.loss.system}件残せませんでした。すべての中断・復帰を記録できていないため、接続状況を確認できません。`:""
  ].filter(Boolean);
  if(!details.length)return "";
  return `終了した録音「${view.title}」：${details.join(" ")} この通知だけで録音ファイルの保存失敗や音声の欠落が分かったわけではありません。会議を開き、音声を再生して内容を確認してください。`;
}
