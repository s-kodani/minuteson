import type { ResetPreview } from "./startup-reset-state";
import { displayNativePathsInText } from "./native-path";
export function StartupRestoreProgress({ message }: { message: string }) {
  return <div className="startup-restore-progress">
    <progress aria-label="データ復元の進捗" aria-valuetext={message || "復元を準備しています"} />
    <p role="status">{displayNativePathsInText(message || "復元を準備しています…")}</p>
  </div>;
}
export function StartupResetForm({ preview, archive, busy, accepted = false, restoring = false, productName = "MinutesOn", selecting, message, error, onArchive, onChoose, onCancel, onStart }: {
  preview: ResetPreview; archive: boolean; busy: boolean; accepted?: boolean; restoring?: boolean; productName?: string; selecting: boolean; message: string; error: string;
  onArchive: (archive: boolean) => void; onChoose: () => void; onCancel: () => void; onStart: () => void;
}) {
  const disabled = busy || selecting, appName = productName;
  if (restoring) return <>
    <h2 id="startup-reset-title">データ復元</h2>
    <p id="startup-reset-description">会議（音声・文字起こし・話者分離・議事録）、ボイス登録、モデル、設定を回収し、初期化後に再登録します。</p>
    <p>すべてのデータを回収・検証できない場合は初期化せず、元データを保持します。</p>
    <dl><dt>対象の保存先</dt><dd style={{ overflowWrap: "anywhere" }}>{displayNativePathsInText(preview.meetingRoot ?? "")}</dd></dl>
    {busy ? <StartupRestoreProgress message={message} /> : message && <p role="status">{displayNativePathsInText(message)}</p>}{error && <p role="alert">{displayNativePathsInText(error)}</p>}
    <div className="analysis-actions"><button type="button" className="secondary" disabled={disabled} onClick={onCancel}>キャンセル</button><button type="button" disabled={disabled || !preview.availability.available} onClick={onStart}>{busy ? "復元中…" : accepted ? `${appName}を再起動` : "復元して再起動"}</button></div>
  </>;
  return <>
    <h2 id="startup-reset-title">データを初期化</h2>
    <p id="startup-reset-description">会議・音声・文書・モデル・設定を初期化し、同じ保存先で{appName}を起動し直します。保存先の選択だけを引き継ぎます。</p>
    <p>モデル本体とダウンロード途中のモデルデータは、退避の有無にかかわらず削除されます。必要なモデルは初期化後に再ダウンロードしてください。</p>
    <dl><dt>対象の保存先</dt><dd style={{ overflowWrap: "anywhere" }}>{displayNativePathsInText(preview.meetingRoot ?? "")}</dd></dl>
    <fieldset className="startup-reset-options" disabled={disabled || accepted}><legend>旧データの退避</legend>
      <label><input type="radio" name="startup-reset-archive" checked={archive} onChange={() => onArchive(true)} />旧データを退避する</label>
      <label><input type="radio" name="startup-reset-archive" checked={!archive} onChange={() => onArchive(false)} />退避しない</label>
    </fieldset>
    {archive && <>
      <p>モデル本体・取得途中データを除いた旧データをフォルダーとして保存します。旧DB内のモデル登録情報は含まれます。通常のバックアップ（.monbackup）とは異なり、アプリからの復元には対応していません。</p>
      <button type="button" className="secondary" disabled={disabled || accepted} onClick={onChoose}>{selecting ? "退避先を選択中…" : "退避先を選択"}</button>
      <dl><dt>作成する退避フォルダー</dt><dd style={{ overflowWrap: "anywhere" }}>{displayNativePathsInText(preview.archivePath ?? "退避先を選択してください")}</dd></dl>
    </>}
    {!archive && <p>旧データは残りません。</p>}
    <p>外部の元音声・書き出し・既存バックアップ、アプリ本体、OSの権限設定は初期化しません。</p>
    {message && <p role="status">{displayNativePathsInText(message)}</p>}{error && <p role="alert">{displayNativePathsInText(error)}</p>}
    <div className="analysis-actions"><button type="button" className="secondary" disabled={disabled} onClick={onCancel}>キャンセル</button><button type="button" disabled={disabled || !preview.availability.available || !accepted && archive && !preview.archivePath} onClick={onStart}>{accepted ? `${appName}を再起動` : "初期化して再起動"}</button></div>
  </>;
}
