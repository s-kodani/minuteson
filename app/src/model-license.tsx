import { useState } from "react";
import { ScopedModal } from "./scoped-modal";
import type { ModelLicenseReview } from "./model-license-state";

export function ModelLicenseDialog({ open, review, loading, error, confirmation, actionLabel, acknowledged, confirming, onAcknowledge, onConfirm, onDismiss, restoreTarget }: {
  open: boolean; review: ModelLicenseReview | null; loading: boolean; error: string; confirmation: boolean; actionLabel: string;
  acknowledged: boolean; confirming: boolean; onAcknowledge: (value: boolean) => void; onConfirm: () => void; onDismiss: () => void; restoreTarget: () => HTMLElement | null;
}) {
  const [copyStatus, setCopyStatus] = useState("");
  const copy = async (source: string) => {
    try { await navigator.clipboard.writeText(source); setCopyStatus("出典をコピーしました。"); }
    catch { setCopyStatus("出典をコピーできませんでした。表示された出典を選択してコピーしてください。"); }
  };
  return <ScopedModal id="model-license-review" title={confirmation ? "利用条件を確認" : "モデルの利用条件"} open={open} onDismiss={onDismiss} restoreTarget={restoreTarget}
    aria-labelledby="model-license-title" className="model-license-dialog" dismissLabel={confirmation ? "キャンセル" : "閉じる"}>
    <h2 id="model-license-title">{confirmation ? "利用条件を確認" : "モデルの利用条件"}</h2>
    {loading && <p role="status">利用条件を読み込んでいます…</p>}
    {error && <p role="alert">{error}</p>}
    {review && <>
      <p className="model-license-label"><strong>{review.modelLabel}</strong></p>
      <p className="model-license-names">モデル・実行エンジン：{review.licenseNames.join(" / ")}</p>
      <p>{review.summary}</p>
      {!!review.restrictions.length && <div className="model-license-restrictions"><h3>{review.official ? "利用にあたっての条件" : "条件未確認"}</h3>{review.restrictions.map((restriction, index) => <p key={index}>{restriction}</p>)}</div>}
      <div className="model-license-notices">{review.notices.map((notice, index) => <details key={`${notice.sha256}-${index}`}>
        <summary>{notice.name}</summary><pre tabIndex={0}>{notice.text}</pre>
        {notice.sourceUrl && <div className="model-license-source"><span>出典：{notice.sourceUrl}</span><button type="button" className="small-secondary" onClick={() => void copy(notice.sourceUrl)}>出典をコピー</button></div>}
      </details>)}</div>
      <p className="model-license-app-note">MinutesOn本体のMIT Licenseと、モデル・実行エンジンの利用条件は別です。全文はオフラインでも確認できます。</p>
      {copyStatus && <p role="status">{copyStatus}</p>}
      {confirmation && <div className="model-license-approval"><label><input type="checkbox" checked={acknowledged} disabled={confirming} onChange={event => onAcknowledge(event.target.checked)} />モデルと実行エンジンの利用条件を確認しました。</label>
        <div className="model-license-actions"><button type="button" className="small-secondary" onClick={onDismiss}>キャンセル</button><button type="button" disabled={!acknowledged || confirming || !!error || loading} onClick={onConfirm}>{confirming ? "選択内容を確認しています…" : actionLabel}</button></div>
      </div>}
      {!confirmation && <div className="model-license-actions"><button type="button" className="small-secondary" onClick={onDismiss}>閉じる</button></div>}
    </>}
  </ScopedModal>;
}
