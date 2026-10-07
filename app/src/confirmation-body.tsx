import { confirmations } from "./confirmation-service";
import type { Confirmation } from "./confirmation-state";

export function ConfirmationBody({ value }: { value: Confirmation<HTMLDialogElement, HTMLElement> }) {
  return <div className="action-confirmation">
    <h2 id={`${value.id}-title`} tabIndex={-1}>{value.title}</h2>
    <p className="confirmation-target">対象: {value.target}</p>
    <p className="confirmation-impact">{value.message}</p>
    {value.phase === "running" && <p role="status">処理中です。完了応答を待っています。</p>}
    {value.phase === "failed" && <p role="alert" className="action-error">処理を完了できませんでした。{value.error} 元の画面で状態を再確認してください。自動再実行はしません。</p>}
    <button type="button" disabled={value.phase !== "choosing"} onClick={() => void confirmations.execute(value.id)}>{value.confirmLabel}</button>
  </div>;
}
