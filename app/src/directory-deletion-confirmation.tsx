import type { RefObject } from "react";

// Presentation only: native operations keep their original, verified paths.
export function deletionDirectoryLabel(directory: string): string {
  if (/^\\\\\?\\UNC\\/iu.test(directory)) return `\\\\${directory.slice(8)}`;
  if (/^\\\\\?\\[A-Za-z]:\\/u.test(directory)) return directory.slice(4);
  return directory;
}

export function DirectoryDeletionConfirmation({ kind, name, directory, titleId, heading, blocked, canDelete, onCancel, onDelete }: {
  kind: "meeting" | "model";
  name: string;
  directory: string;
  titleId: string;
  heading: RefObject<HTMLHeadingElement | null>;
  blocked: boolean;
  canDelete: boolean;
  onCancel: () => void;
  onDelete: () => void;
}) {
  return <>
    <h2 id={titleId} ref={heading} tabIndex={-1}>{kind === "meeting" ? "会議を削除" : "モデルを削除"}</h2>
    <p>{kind === "meeting" ? `「${name}」の音声・文字起こし・議事録などを完全に削除しますか？` : `「${name}」のローカルコピーを完全に削除しますか？`}</p>
    {kind === "meeting" && <p>ボイス登録用に保存した音声と声の特徴量は残ります。</p>}
    <div><span>削除対象フォルダー</span><p className="data-path" style={{ overflowWrap: "anywhere", whiteSpace: "pre-wrap" }}>{deletionDirectoryLabel(directory)}</p></div>
    <div className="analysis-actions">
      <button type="button" className="secondary" data-dialog-cancel disabled={blocked} onClick={onCancel}>キャンセル</button>
      <button type="button" className="stop-button" disabled={blocked || !canDelete} onClick={onDelete}>削除</button>
    </div>
  </>;
}
