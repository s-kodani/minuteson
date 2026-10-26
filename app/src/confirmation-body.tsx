import { ErrorNotice } from "./toast";
import { confirmations } from "./confirmation-service";
import type { Confirmation } from "./confirmation-state";

export function ConfirmationBody({ value }: { value: Confirmation<HTMLDialogElement, HTMLElement> }) {
  return <div className="action-confirmation">
    <h2 id={`${value.id}-title`} tabIndex={-1}>{value.title}</h2>
    <p className="confirmation-target">対象: {value.target}</p>
    <p className="confirmation-impact">{value.message}</p>
    {value.phase === "running" && <p role="status">処理中です。完了応答を待っています。</p>}
    {value.phase === "running" && value.progressLabel && <><button type="button" disabled={!!value.progressPending || !!value.progressDone} onClick={() => void confirmations.runProgressAction(value.id)}>{value.progressLabel}</button>{value.progressDone && <p role="status">停止を要求しました。処理の終了応答を待っています。</p>}<ErrorNotice scope={`confirmation:${value.id}`} type="ConfirmationBody-progress" active={!!(value.progressError)}>停止要求を確認できませんでした。{value.progressError} 処理の終了を待つか、明示的に停止要求を再試行してください。</ErrorNotice></>}
    <ErrorNotice scope={`confirmation:${value.id}`} type="ConfirmationBody-operation" active={!!(value.phase === "failed")}>処理を完了できませんでした。{value.error} 元の画面で状態を再確認してください。自動再実行はしません。</ErrorNotice>
    <div className="confirmation-actions">
      {value.alternativeLabel && <button type="button" className="small-secondary" disabled={value.phase !== "choosing"} onClick={() => void confirmations.execute(value.id, true)}>{value.alternativeLabel}</button>}
      <button type="button" disabled={value.phase !== "choosing"} onClick={() => void confirmations.execute(value.id)}>{value.confirmLabel}</button>
    </div>
  </div>;
}
