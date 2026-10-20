import { IconButton } from "./action-icon";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { rowMenuReturnTarget } from "./row-menu";
import { IssueNotice, ToastNotice } from "./toast";
import { CommonModal } from "./common-modal";
import { useEffect, useRef, useState } from "react";
import type { DataOperation } from "./data-operation";
import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state";
import { assertModelDeletionPage, modelDeletionPhaseLabel, type ModelDeletionPage, type ModelDeletionReceipt } from "./model-deletion-operation-state";
import { bytesLabel } from "./import-state";
import { stopAllPlayback } from "./playback-barrier";

export function useModelDeletion({ operation, locked, withSavedEdits, onDeleted }: {
  operation: DataOperation; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void | "cancelled">;
  onDeleted: (receipt: ModelDeletionReceipt) => Promise<void>;
}) {
  const latest = useRef({ operation, locked, withSavedEdits, onDeleted }); latest.current = { operation, locked, withSavedEdits, onDeleted };
  const [rpcBusy, setRpcBusy] = useState(false), running = useRef(false);
  const [unknown, setUnknown] = useState(false), [open, setOpen] = useState(false);
  const [error, setError] = useState(""), [toast, setToast] = useState("");
  const [toastEvent, setToastEvent] = useState("");
  const [page, setPage] = useState<ModelDeletionPage | null>(null), [pageError, setPageError] = useState("");
  const [pageBusy, setPageBusy] = useState(false), pageEpoch = useRef(0);
  const [understood, saveUnderstood] = useState(false), [includeUnknown, saveIncludeUnknown] = useState(false);
  const [understoodId, setUnderstoodId] = useState<string | null>(null), [includeUnknownId, setIncludeUnknownId] = useState<string | null>(null);
  const pending = useRef<{ request: DataRequest; revision: number } | null>(null);
  const choice = useRef<{ id: string; pending: boolean; revision: number } | null>(null);
  const completed = useRef<string | null>(null), opened = useRef<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null), heading = useRef<HTMLHeadingElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const s = operation.snapshot, preview = s?.kind === "model_deletion" ? s.modelDeletionPreview : null;
  const receipt = s?.kind === "model_deletion" ? s.modelDeletionReceipt : null;
  const currentUnderstood = understood && understoodId === preview?.confirmationId;
  const currentIncludeUnknown = includeUnknown && includeUnknownId === preview?.confirmationId;
  const setUnderstood = (value: boolean) => { saveUnderstood(value); setUnderstoodId(preview?.confirmationId ?? null); };
  const setIncludeUnknown = (value: boolean) => { saveIncludeUnknown(value); setIncludeUnknownId(preview?.confirmationId ?? null); };
  const listing = operation.modelDeletionListing;
  const protectedNow = rpcBusy || unknown;
  const canInspect = isTauri() && operation.modelDeletionSupported && operation.connected && !locked && !rpcBusy && !unknown && !s?.busy;
  // Restart normally forbids all work. A canonical pending deletion alone can
  // use its restricted native resume lane; it never enables ordinary writes.
  const sharedAvailable = !!operation.availability?.available || (!!listing?.restartRequired && operation.availability?.reason === "保存先の切替後は終了して再起動が必要です");
  const canReviewPending = canInspect && sharedAvailable;
  const canReviewModel = canReviewPending && !listing?.writeBlocked && !operation.deletionListing?.writeBlocked && !s?.deletionPreview?.pending && !operation.audioDeletionListing?.writeBlocked && !s?.audioDeletionPreview?.pending && !s?.restartRequired && !preview?.pending;
  const canResolve = canReviewPending && !!preview && !receipt && (!s?.restartRequired || preview.pending);
  const run = async (action: () => Promise<void>) => {
    if (running.current) return;
    running.current = true; setRpcBusy(true); setError("");
    try { await action(); } catch (cause) { setError(String(cause)); }
    finally { running.current = false; setRpcBusy(false); }
  };
  const protect = async (action: () => Promise<void>) => {
    // Pending receipts already fenced writes. Initial review/publication flush
    // edits before that fence; recovery never attempts to rewrite deleted rows.
    if (latest.current.operation.deletionListing?.writeBlocked || latest.current.operation.modelDeletionListing?.writeBlocked || latest.current.operation.audioDeletionListing?.writeBlocked || latest.current.operation.getSnapshot()?.audioDeletionPreview?.pending || latest.current.operation.getSnapshot()?.modelDeletionPreview?.pending) await run(action);
    else await latest.current.withSavedEdits(async () => run(action));
  };
  const loadPage = async (offset = 0) => {
    const p = latest.current.operation.getSnapshot()?.modelDeletionPreview;
    if (!p || latest.current.operation.getSnapshot()?.busy) return;
    const epoch = ++pageEpoch.current;
    setPage(null); setPageError(""); setPageBusy(true);
    try {
      const value = await invoke<ModelDeletionPage>("get_model_deletion_paths", { confirmationId: p.confirmationId, offset });
      assertModelDeletionPage(value, p, offset);
      if (epoch === pageEpoch.current && latest.current.operation.getSnapshot()?.modelDeletionPreview?.confirmationId === p.confirmationId) setPage(value);
    } catch (cause) { if (epoch === pageEpoch.current) setPageError(String(cause)); }
    finally { if (epoch === pageEpoch.current) setPageBusy(false); }
  };
  const review = async (id: string, pendingReview: boolean, reviewHash?: string) => {
    if (!(pendingReview ? canReviewPending : canReviewModel)) return;
    const active = rowMenuReturnTarget(document.activeElement instanceof HTMLElement ? document.activeElement : null);
    await protect(async () => {
      await stopAllPlayback(); opener.current = active; setOpen(false); setToast("");
      choice.current = { id, pending: pendingReview, revision: latest.current.operation.getSnapshot()?.revision ?? -1 };
      try {
        latest.current.operation.accept(await invoke<DataSnapshot>(pendingReview ? "review_pending_model_deletion" : "review_model_body_deletion", pendingReview ? { operationId: id } : { packageId: id, expectedReviewHash: reviewHash }));
        choice.current = null; setUnknown(false);
      } catch (cause) { setUnknown(true); throw Error(`確認の受付を確定できません。削除は実行せず、同じ確認の状態を読み直してください: ${String(cause)}`); }
    });
  };
  const send = async (request: DataRequest, revision: number) => {
    pending.current = { request, revision };
    try { latest.current.operation.accept(await invoke<DataSnapshot>("start_data_operation", { request })); pending.current = null; setUnknown(false); setOpen(false); }
    catch (cause) { setUnknown(true); throw Error(`受付結果を確定できません。別の削除を始めず、同じ操作の結果を確認してください: ${String(cause)}`); }
  };
  const submit = async (action: DataRequest["action"]) => {
    if (!canResolve) return;
    const confirmation = preview?.confirmationId;
    if (!confirmation) return;
    if (["publish", "delete_including_unregistered"].includes(action) && (!open || !currentUnderstood || !page || pageBusy || pageError || page.confirmationId !== preview?.confirmationId || (action === "delete_including_unregistered" && !currentIncludeUnknown))) return;
    await protect(async () => {
      await stopAllPlayback();
      const now = latest.current.operation.getSnapshot(); if (!now) throw Error("削除状態を再確認してください。");
      if (now.modelDeletionPreview?.confirmationId !== confirmation) throw Error("待機中に削除の確認が変更されました。新しい一覧を確認してください。");
      await send(dataRequest(now, action), now.revision);
    });
  };
  const retryUnknown = async () => {
    if (running.current || locked || s?.busy) return;
    await protect(async () => {
      if (pending.current) { const p = pending.current; await stopAllPlayback(); await send(p.request, p.revision); }
      else { await latest.current.operation.refresh(); const now = latest.current.operation.getSnapshot(), c = choice.current;
        if (c && now?.kind === "model_deletion" && now.revision > c.revision && (now.busy || ["failed", "cancelled", "attention"].includes(now.phase) || (c.pending ? now.modelDeletionPreview?.operationId === c.id : now.modelDeletionPreview?.packageId === c.id))) { choice.current = null; setUnknown(false); setError(""); }
        else throw Error("同じ確認の受付をまだ確定できません。読み直すか、安全な停止を要求してください。");
      }
    });
  };
  const cancel = async () => {
    const now = latest.current.operation.getSnapshot();
    if (running.current || now?.kind !== "model_deletion" || !now.token) return;
    await run(async () => { latest.current.operation.accept(await invoke<DataSnapshot>("cancel_data_operation", { token: now.token })); pending.current = null; choice.current = null; setUnknown(false); setOpen(false); });
  };
  const dismiss = async () => {
    if (rpcBusy || unknown || s?.busy) return;
    if (preview?.pending || receipt) setOpen(false); else await cancel();
  };
  const quit = async () => { if (!rpcBusy && !s?.busy && !unknown) await run(async () => { await invoke("finish_main_close", { quit: true }); }); };
  useEffect(() => {
    setUnderstood(false); setIncludeUnknown(false); pageEpoch.current++; setPage(null); setPageError("");
    if (preview && !s?.busy) void loadPage();
    return () => { pageEpoch.current++; };
  }, [preview?.confirmationId, s?.busy]);
  useEffect(() => {
    if (!s || s.kind !== "model_deletion") { setOpen(false); return; }
    if (receipt || ["idle", "cancelled"].includes(s.phase)) setOpen(false);
    else if (s.busy) setOpen(true);
    if (unknown) {
      const c = choice.current, p = pending.current;
      if ((c && s.revision > c.revision && (s.busy || ["failed", "cancelled", "attention"].includes(s.phase) || (c.pending ? preview?.operationId === c.id : preview?.packageId === c.id)))
        || (p && s.token === p.request.token && s.revision > p.revision && (s.busy || ["completed", "failed", "cancelled", "attention"].includes(s.phase) || (s.phase === "ready" && preview?.confirmationId !== p.request.confirmationId)))) { pending.current = null; choice.current = null; setUnknown(false); setError(""); }
    }
    if (s.phase === "ready" && preview && !rpcBusy && !locked && !unknown && opened.current !== preview.confirmationId) { opened.current = preview.confirmationId; setOpen(true); }
    if (receipt && completed.current !== s.token) {
      completed.current = s.token; setToastEvent(s.token ?? ""); setError("");
      setToast(receipt.bodyDeleted ? "モデルをローカルから削除しました。" : "ファイル削除を開始せず保持して中止しました。部分削除の復元ではありません。");
      if (receipt.bodyDeleted) void latest.current.onDeleted(receipt).catch(cause => setError(`削除は完了しています。モデル一覧を再読込してください: ${String(cause)}`));
      void latest.current.operation.refresh().catch(cause => setError(String(cause)));
    }
  }, [s, rpcBusy, locked, unknown]);
  const canDelete = canResolve && open && s?.phase === "ready" && preview?.executeAllowed && currentUnderstood && !!page && page.confirmationId === preview.confirmationId && !pageBusy && !pageError && (!preview.unregisteredCount || !["review", "not_started", "reconfirmation"].includes(preview.phase) || currentIncludeUnknown);
  return { reviewModel: (id: string, hash: string) => review(id, false, hash), reviewPending: (id: string) => review(id, true), canReviewModel, canReviewPending, canResolve,
    rpcBusy, unknown, protectedNow, open, setOpen, preview, receipt, listing, error, toast, toastEvent, setToast, page, pageBusy, pageError, loadPage,
    understood: currentUnderstood, setUnderstood, includeUnknown: currentIncludeUnknown, setIncludeUnknown, canDelete, submit, retryUnknown, cancel, dismiss, quit, dialog, heading, opener };
}

export type ModelDeletionControl = ReturnType<typeof useModelDeletion>;

export function ModelDeletionDialog({ operation, control: c }: { operation: DataOperation; control: ModelDeletionControl }) {
  const s = operation.snapshot, p = c.preview, active = s?.kind === "model_deletion";
  const busy = active && !!s?.busy;
  const checking = busy && ["choosing", "verifying", "verifying_model_delete_scope"].includes(s.phase);
  return <>
    <ToastNotice scope="model-deletion" type="result" eventId={c.toastEvent} message={c.toast} onClose={() => c.setToast("")} />
    <IssueNotice scope="model-deletion" type="model-deletion" label="モデル削除" message={c.error || (active ? operation.error || (s.phase === "cancelled" ? "" : s.errorMessage) : "")} />
    <CommonModal portal ref={c.dialog} open={c.open} blocked={c.protectedNow || !!s?.busy} onDismiss={c.dismiss}
      restoreReady={!c.rpcBusy && !s?.busy && operation.connected}
      restoreTarget={() => c.opener.current?.isConnected && !c.opener.current.matches(":disabled") ? c.opener.current : document.querySelector<HTMLElement>(".model-inventory h2")}
      className="import-dialog data-dialog deletion-dialog" aria-labelledby="model-deletion-title">
      <h2 id="model-deletion-title" ref={c.heading} tabIndex={-1}>{busy && s.phase === "cancelling" ? "モデル操作の停止を待機中" : checking ? "モデルの削除対象を確認中" : busy ? "モデル削除の処理中" : p?.phase === "reconfirmation" ? "残りのモデルファイルを確認" : "モデルのローカル削除を確認"}</h2>
      {busy && <div className="model-deletion-progress"><p role="status">{s.phase === "cancelling" ? "安全な停止を待っています。" : checking ? "モデルファイルの内容と削除範囲を検査しています。この確認処理ではファイルを削除しません。" : "確認したモデルファイルの削除を処理しています。"}</p>
        {checking && <p>容量の大きいモデルは確認に時間がかかります。</p>}
        <button type="button" className="stop-button" disabled={c.rpcBusy || s.phase === "cancelling"} onClick={() => void c.cancel()}>{checking ? "確認を中止する" : "安全な停止を要求"}</button>
      </div>}
      {p && !busy && <><h3>{p.modelName}</h3><p className="data-path">パッケージID: {p.packageId}</p>
        <p>このコピーのモデル一式を削除します。モデル本体のほか、このコピーに含まれる実行プログラム・発話検知モデル・付属文書も対象です。この操作は取り消せません。会議データ・設定・処理履歴は保持します。</p>
        <p>選択中のモデルを削除すると、再導入または別モデルへの切替まで、そのモデルを使うAI処理は利用できません。</p>
        <dl className="import-info"><dt>対象ファイル・容量</dt><dd>{p.fileCount} 件 · {bytesLabel(p.bytes)}</dd>
          <dt>欠落・変更・未登録</dt><dd>{p.missingCount} / {p.changedCount} / {p.unregisteredCount} 件</dd>
          <dt>段階</dt><dd>{modelDeletionPhaseLabel(p.phase)}</dd>
          <dt>処理済みパス</dt><dd>{p.processedPaths} / {p.pathCount}</dd></dl>
        {p.pending && <p role="alert">途中まで削除している可能性があります。消したファイルは復元しません。</p>}
        {p.phase === "reconfirmation" && <p>以前の計画では {p.previousProcessedPaths} / {p.previousPathCount} パスを処理済みです。現在残る範囲に新しく同意して続行します。</p>}
        {p.resumeReason && <p role="alert">{p.resumeReason}</p>}
        {p.reconfirmationAllowed && <button type="button" disabled={!c.canResolve} onClick={() => void c.submit("review_remaining")}>変更後の残り範囲を確認する</button>}
        {c.pageBusy && <p role="status">対象ファイルを確認中…</p>}
        {c.pageError && <p role="alert">{c.pageError}</p>}
        {c.page && <><ul className="deletion-paths">{c.page.entries.map(e => <li key={e.relativePath}><code>{e.relativePath}</code><span> — {e.directory ? "フォルダー" : e.present ? bytesLabel(e.bytes ?? 0) : "欠落"}{e.changed ? " · 変更あり" : ""}{!e.registered ? " · 未登録" : ""}</span></li>)}</ul>
          <div className="analysis-actions"><IconButton label="前の50件" icon="previous" type="button" className="small-secondary" disabled={!c.canResolve || c.page.offset === 0} onClick={() => void c.loadPage(Math.max(0, c.page!.offset - 50))} /><IconButton label="次の50件" icon="next" type="button" className="small-secondary" disabled={!c.canResolve || c.page.nextOffset === null} onClick={() => void c.loadPage(c.page!.nextOffset!)} /></div></>}
        {!!p.unregisteredCount && ["review", "not_started", "reconfirmation"].includes(p.phase) && <label className="deletion-consent"><input type="checkbox" checked={c.includeUnknown} disabled={!c.canResolve} onChange={e => c.setIncludeUnknown(e.target.checked)} />一覧の未登録ファイルも削除する</label>}
        <label className="deletion-consent"><input type="checkbox" checked={c.understood} disabled={!c.canResolve} onChange={e => c.setUnderstood(e.target.checked)} />対象コピーと削除の影響を確認しました</label>
      </>}
      {!busy && <div className="analysis-actions"><button type="button" className="danger-button" disabled={!c.canDelete} onClick={() => void c.submit(p?.unregisteredCount && ["review", "not_started", "reconfirmation"].includes(p.phase) ? "delete_including_unregistered" : "publish")}>{p?.pending ? "残りの削除を続行" : "ローカルから削除する"}</button>
        <button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => void c.submit("refresh")}>残りの状態を再検査</button>
        {p?.keepAllowed && p.pending && <button type="button" disabled={!c.canResolve} onClick={() => void c.submit("keep")}>未開始のまま保持して中止</button>}
        <button type="button" className="small-secondary" data-dialog-cancel disabled={c.rpcBusy || !!s?.busy || c.unknown} onClick={() => void c.dismiss()}>削除せず閉じる</button></div>}
      {c.unknown && <div className="analysis-actions"><button type="button" disabled={c.rpcBusy || !!s?.busy} onClick={() => void c.retryUnknown()}>同じ削除操作の結果を確認</button><button type="button" disabled={c.rpcBusy || !s?.token} onClick={() => void c.cancel()}>同じ操作の安全な停止を要求</button></div>}
      {(c.error || (active && s.errorMessage)) && <p className="action-error" role="alert">{c.error || s?.errorMessage}</p>}
    </CommonModal>
  </>;
}
