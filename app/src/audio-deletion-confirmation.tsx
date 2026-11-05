import type { RefObject } from "react";

export function AudioDeletionConfirmation({ name, heading, blocked, canDelete, onCancel, onDelete }: {
  name: string;
  heading: RefObject<HTMLHeadingElement | null>;
  blocked: boolean;
  canDelete: boolean;
  onCancel: () => void;
  onDelete: () => void;
}) {
  return <>
    <h2 id="audio-deletion-title" ref={heading} tabIndex={-1}>音声だけを削除</h2>
    <p className="data-path">「{name}」の音声だけを削除しますか？</p>
    <p>音声は復元できません。文字起こし・議事録・下書きは保持します。ボイス登録用に保存した音声と声の特徴量も残ります。</p>
    <div className="analysis-actions">
      <button type="button" className="secondary" data-dialog-cancel disabled={blocked} onClick={onCancel}>キャンセル</button>
      <button type="button" className="stop-button" disabled={blocked || !canDelete} onClick={onDelete}>削除</button>
    </div>
  </>;
}
