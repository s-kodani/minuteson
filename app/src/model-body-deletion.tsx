import { IconButton } from "./action-icon";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { quietMaintenance } from "./maintenance-visibility";
import { InfoPopover } from "./info-popover";
import { IssueNotice, ToastNotice } from "./toast";
import { CommonModal } from "./common-modal";
import { useEffect, useRef, useState } from "react";
import type { DataOperation } from "./data-operation";
import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state";
import { assertModelBodyDeletionPage, modelDeletionStartupLabel, type ModelBodyDeletionPage, type ModelBodyDeletionReceipt } from "./model-deletion-operation-state";
import { modelDeletionNewScope, modelDeletionPathProcessed, modelDeletionPhaseLabel, modelDeletionPublishRequest } from "./model-deletion-ui-state";
import { bytesLabel } from "./import-state";
import { stopAllPlayback } from "./playback-barrier";

export function useModelBodyDeletion({ operation, locked, withSavedEdits, onDeleted, onProtectionChange }: {
  operation: DataOperation; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  onDeleted: (receipt: ModelBodyDeletionReceipt) => Promise<void>;
  onProtectionChange: (protectedNow: boolean) => void;
}) {
  const latest = useRef({ operation, locked, withSavedEdits, onDeleted, onProtectionChange }); latest.current = { operation, locked, withSavedEdits, onDeleted, onProtectionChange };
  const running = useRef(false), [rpcBusy, setRpcBusy] = useState(false);
  const [unknown, setUnknown] = useState(false), [open, setOpen] = useState(false);
  const [error, setError] = useState(""), [toast, setToast] = useState("");
  const [toastEvent, setToastEvent] = useState("");
  const [page, setPage] = useState<ModelBodyDeletionPage | null>(null), [pageError, setPageError] = useState(""), [pageBusy, setPageBusy] = useState(false);
  const pageEpoch = useRef(0), [understoodId, setUnderstoodId] = useState<string | null>(null), [includeUnknownId, setIncludeUnknownId] = useState<string | null>(null);
  const pending = useRef<{ request: DataRequest; revision: number } | null>(null);
  const choice = useRef<{ id: string; pending: boolean; revision: number } | null>(null);
  const completed = useRef<string | null>(null), opened = useRef<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null), heading = useRef<HTMLHeadingElement>(null), opener = useRef<HTMLElement | null>(null);
  const s = operation.snapshot, preview = s?.kind === "model_deletion" ? s.modelDeletionPreview : null, receipt = s?.kind === "model_deletion" ? s.modelDeletionReceipt : null;
  const listing = operation.modelDeletionListing, protectedNow = rpcBusy || unknown;
  const sharedAvailable = !!operation.availability?.available || (!!listing?.restartRequired && operation.availability?.reason === "保存先の切替後は終了して再起動が必要です");
  const canReviewPending = isTauri() && operation.modelDeletionSupported && operation.connected && sharedAvailable && !locked && !protectedNow && !s?.busy;
  const canReviewNew = canReviewPending && !listing?.writeBlocked && !operation.deletionListing?.writeBlocked && !operation.audioDeletionListing?.writeBlocked && !s?.audioDeletionPreview?.pending && !s?.restartRequired && !preview?.pending && !s?.deletionPreview?.pending;
  const canResolve = canReviewPending && !!preview && !receipt && (!s?.restartRequired || preview.pending);
  const understood = !!preview && understoodId === preview.confirmationId, includeUnknown = !!preview && includeUnknownId === preview.confirmationId;
  const run = async (action: () => Promise<void>) => {
    if (running.current) return;
    running.current = true; setRpcBusy(true); setError("");
    try { await action(); } catch (cause) { setError(String(cause)); }
    finally { running.current = false; setRpcBusy(false); }
  };
  const protect = async (action: () => Promise<void>) => {
    const op = latest.current.operation, now = op.getSnapshot();
    if (op.modelDeletionListing?.writeBlocked || op.deletionListing?.writeBlocked || op.audioDeletionListing?.writeBlocked || now?.audioDeletionPreview?.pending || now?.modelDeletionPreview?.pending || now?.deletionPreview?.pending) await run(action);
    else await latest.current.withSavedEdits(async () => run(action));
  };
  const loadPage = async (offset = 0) => {
    const now = latest.current.operation.getSnapshot(), p = now?.kind === "model_deletion" ? now.modelDeletionPreview : null;
    if (!p || now?.busy) return;
    const epoch = ++pageEpoch.current; setPage(null); setPageError(""); setPageBusy(true);
    try {
      const value = await invoke<ModelBodyDeletionPage>("get_model_deletion_paths", { confirmationId: p.confirmationId, offset });
      assertModelBodyDeletionPage(value, p, offset);
      if (epoch === pageEpoch.current && latest.current.operation.getSnapshot()?.modelDeletionPreview?.confirmationId === p.confirmationId) setPage(value);
    } catch (cause) { if (epoch === pageEpoch.current) setPageError(String(cause)); }
    finally { if (epoch === pageEpoch.current) setPageBusy(false); }
  };
  const review = async (id: string, reviewHash: string | null) => {
    if (!(reviewHash === null ? canReviewPending : canReviewNew)) return;
    const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    await protect(async () => {
      await stopAllPlayback(); opener.current = active; setOpen(false); setToast("");
      choice.current = { id, pending: reviewHash === null, revision: latest.current.operation.getSnapshot()?.revision ?? -1 };
      try {
        latest.current.operation.accept(await invoke<DataSnapshot>(reviewHash === null ? "review_pending_model_deletion" : "review_model_body_deletion", reviewHash === null ? { operationId: id } : { packageId: id, expectedReviewHash: reviewHash }));
        choice.current = null; setUnknown(false);
      } catch (cause) { setUnknown(true); throw Error(`確認の受付を確定できません。削除は実行せず、同じ確認の状態を読み直してください: ${String(cause)}`); }
    });
  };
  const send = async (request: DataRequest, revision: number) => {
    pending.current = { request, revision };
    try { latest.current.operation.accept(await invoke<DataSnapshot>("start_data_operation", { request })); pending.current = null; setUnknown(false); setOpen(false); }
    catch (cause) { setUnknown(true); throw Error(`受付結果を確定できません。別の削除を始めず、同じモデル操作の結果を確認してください: ${String(cause)}`); }
  };
  const submit = async (action: DataRequest["action"]) => {
    if (!canResolve || !preview) return;
    const confirmation = preview.confirmationId, token = s?.token, publishing = action === "publish";
    if (publishing && (!open || !understood || !page || pageBusy || pageError)) return;
    await protect(async () => {
      await stopAllPlayback();
      const now = latest.current.operation.getSnapshot();
      if (!now || now.kind !== "model_deletion" || now.token !== token || now.modelDeletionPreview?.confirmationId !== confirmation || !latest.current.operation.connected) throw Error("待機中にモデル削除の確認が変更されました。新しい状態を確認してください。");
      const request = publishing ? modelDeletionPublishRequest(now, confirmation, page, understoodId, includeUnknownId) : dataRequest(now, action);
      await send(request, now.revision);
    });
  };
  const matchesChoice = (now: DataSnapshot, c: NonNullable<typeof choice.current>) => now.kind === "model_deletion" && now.revision > c.revision && (now.busy || ["failed", "cancelled", "attention"].includes(now.phase) || (c.pending ? now.modelDeletionPreview?.operationId === c.id : now.modelDeletionPreview?.packageId === c.id));
  const retryUnknown = async () => {
    if (running.current || locked || s?.busy) return;
    await protect(async () => {
      if (pending.current) { const p = pending.current; await stopAllPlayback(); await send(p.request, p.revision); }
      else {
        await latest.current.operation.refresh(); const now = latest.current.operation.getSnapshot(), c = choice.current;
        if (now && c && matchesChoice(now, c)) { choice.current = null; setUnknown(false); }
        else throw Error("同じ確認の受付をまだ確定できません。読み直すか、安全な停止を要求してください。");
      }
    });
  };
  const cancel = async () => {
    const now = latest.current.operation.getSnapshot();
    if (running.current || now?.kind !== "model_deletion" || !now.token) return;
    await run(async () => { latest.current.operation.accept(await invoke<DataSnapshot>("cancel_data_operation", { token: now.token })); pending.current = null; choice.current = null; setUnknown(false); setOpen(false); });
  };
  const dismiss = async () => { if (!protectedNow && !s?.busy) { if (preview?.pending || receipt) setOpen(false); else await cancel(); } };
  const quit = async () => { if (!protectedNow && !s?.busy) await run(async () => { await invoke("finish_main_close", { quit: true }); }); };
  useEffect(() => { latest.current.onProtectionChange(protectedNow); return () => { latest.current.onProtectionChange(false); }; }, [protectedNow]);
  useEffect(() => {
    setUnderstoodId(null); setIncludeUnknownId(null); pageEpoch.current++; setPage(null); setPageError("");
    if (preview && !s?.busy) void loadPage();
    return () => { pageEpoch.current++; };
  }, [preview?.confirmationId, s?.busy]);
  useEffect(() => {
    if (!s || s.kind !== "model_deletion") { setOpen(false); return; }
    if (s.busy || receipt) setOpen(false);
    if (unknown) {
      const c = choice.current, p = pending.current;
      if ((c && matchesChoice(s, c)) || (p && s.token === p.request.token && s.revision > p.revision && (s.busy || ["completed", "failed", "cancelled", "attention"].includes(s.phase) || (s.phase === "ready" && preview?.confirmationId !== p.request.confirmationId)))) { pending.current = null; choice.current = null; setUnknown(false); }
    }
    if (s.phase === "ready" && preview && !rpcBusy && !locked && !unknown && opened.current !== preview.confirmationId) { opened.current = preview.confirmationId; setOpen(true); }
    if (receipt && completed.current !== s.token) {
      completed.current = s.token; setToastEvent(s.token ?? ""); setError("");
      setToast(receipt.bodyDeleted ? "モデル本体を削除しました。会議データ・モデル構成と処理の履歴は保持しています。再利用には本体の準備が必要です。" : "モデル本体の削除を開始せず保持して中止しました。部分削除の復元ではありません。");
      if (receipt.bodyDeleted) void latest.current.onDeleted(receipt).catch(cause => setError(`本体削除は完了しています。配置記録を再読込してください: ${String(cause)}`));
      void latest.current.operation.refresh().catch(cause => setError(String(cause)));
    }
  }, [s, rpcBusy, locked, unknown]);
  let canDelete = false;
  if (canResolve && open && understood && page && !pageBusy && !pageError && s && preview) {
    try { modelDeletionPublishRequest(s, preview.confirmationId, page, understoodId, includeUnknownId); canDelete = true; } catch { /* Fail closed on stale consent/page/phase. */ }
  }
  return { reviewNew: (id: string, reviewHash: string) => review(id, reviewHash), reviewPending: (id: string) => review(id, null), canReviewNew, canReviewPending, canResolve,
    protectedNow, rpcBusy, unknown, open, setOpen, error, toast, toastEvent, setToast, preview, receipt, listing, page, pageBusy, pageError, loadPage,
    understood, setUnderstood: (v: boolean) => setUnderstoodId(v ? preview?.confirmationId ?? null : null), includeUnknown, setIncludeUnknown: (v: boolean) => setIncludeUnknownId(v ? preview?.confirmationId ?? null : null),
    canDelete, submit, cancel, dismiss, retryUnknown, quit, dialog, heading, opener };
}
export type ModelBodyDeletionControl = ReturnType<typeof useModelBodyDeletion>;

export function ModelBodyDeletionPanel({ operation, control: c, visible, compact = false }: { operation: DataOperation; control: ModelBodyDeletionControl; visible: boolean; compact?: boolean }) {
  const s = operation.snapshot, p = c.preview, active = s?.kind === "model_deletion", needed = !!c.listing?.writeBlocked || !!p?.pending;
  const quiet = quietMaintenance({ compact, connected: operation.connected, supported: operation.modelDeletionSupported, listed: !!c.listing, itemCount: c.listing?.items.length ?? 0, attention: !!(active || needed || c.error || c.protectedNow || c.open || c.listing?.restartRequired || c.receipt?.restartRequired) });
  return <>
    <ToastNotice scope="models" type="body-deletion-result" eventId={c.toastEvent} message={c.toast} onClose={() => c.setToast("")} />
    {(visible && !quiet || active || needed || c.error || c.protectedNow || (isTauri() && !operation.connected)) && <section className="analysis-card deletion-panel" aria-label="モデル本体削除と中断対応">
      <div className="heading-with-info"><h2 id="model-body-deletion-heading" tabIndex={-1}>モデル本体削除と中断対応</h2><InfoPopover label="モデル本体削除" details={<p>端末内の指定モデル本体だけを削除します。会議音声・本文・議事録・モデル名とSHA・構成履歴・ジョブ履歴・現在の設定は保持します。本体を失った構成は、再準備まで利用できません。外部の導入元、他モデル、バックアップは対象外です。</p>}>指定した管理モデル本体だけを削除します。会議と構成履歴は残りますが、対応するAI処理にはモデルの再準備が必要です。</InfoPopover></div>
      {!operation.modelDeletionSupported && operation.connected && <p>この環境のモデル本体削除は保留中です。</p>}
      {!operation.connected && <p role="alert">状態を確認できるまで変更を停止しています。{operation.error}</p>}
      <IconButton label="モデル削除と書込停止の状態を再読込" icon="refresh" type="button" className="small-secondary" disabled={c.rpcBusy} onClick={() => void operation.refresh().catch(() => {})} />
      {needed && <p role="alert">編集・録音・AI処理などの通常書込みを停止しています。{c.listing?.reason}</p>}
      {!!c.listing?.items.length && <ul>{c.listing.items.map(i => <li key={i.operationId}><p>{i.modelName ?? "確認できないモデル"} · 操作 {i.operationId} · 記録状態 {i.startupReason ? "完了記録あり・対象確認が必要" : i.state}</p>{i.startupReason && <p role="alert">{modelDeletionStartupLabel(i.startupReason)}</p>}{i.errorMessage && <p role="alert">{i.errorMessage}</p>}<button type="button" className="small-secondary" disabled={!i.reviewAllowed || !c.canReviewPending} onClick={() => void c.reviewPending(i.operationId)}>未完了モデル削除の対象を確認</button></li>)}</ul>}
      {active && <p role="status">{s.busy ? "モデル本体の検査・処理中です。停止要求後も実際の終了を待ちます。" : p ? modelDeletionPhaseLabel(p.phase) : "モデル削除の確認待ちです。"}</p>}
      {active && s.busy && <button type="button" className="stop-button" disabled={c.rpcBusy || s.phase === "cancelling"} onClick={() => void c.cancel()}>モデル削除の安全な停止を要求</button>}
      {p && !s?.busy && !c.receipt && <div className="analysis-actions"><button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => { c.opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; c.setOpen(true); }}>モデル削除の確認内容を開く</button><button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => void c.submit("refresh")}>モデル本体の残りを再検査</button></div>}
      {c.unknown && <button type="button" disabled={c.rpcBusy || !!s?.busy} onClick={() => void c.retryUnknown()}>同じモデル削除操作の結果を確認</button>}
      <IssueNotice scope="models" type="body-deletion" label="モデル削除" message={c.error || (active ? s.errorMessage : "")} />
      {(c.listing?.restartRequired || c.receipt?.restartRequired) && <><p role="alert">通常利用にはアプリの再起動が必要です。未確認の応答は先に同じ操作で確認してください。</p><button type="button" disabled={c.protectedNow || !!s?.busy} onClick={() => void c.quit()}>終了する（再起動は手動）</button></>}
    </section>}
    <CommonModal ref={c.dialog} open={c.open} blocked={c.protectedNow || !!s?.busy} onDismiss={c.dismiss} restoreReady={!c.rpcBusy && !s?.busy && operation.connected} restoreTarget={() => c.opener.current?.isConnected && !c.opener.current.matches(":disabled") ? c.opener.current : document.getElementById("model-body-deletion-heading")} className="import-dialog data-dialog deletion-dialog" aria-labelledby="model-body-deletion-title">
      <h2 id="model-body-deletion-title" ref={c.heading} tabIndex={-1}>{p?.phase === "reconfirmation" ? "モデル本体の残り範囲への再同意" : "モデル本体の削除を確認"}</h2>
      {p && <><h3>{p.modelName} / {p.modelVersion}</h3><p className="data-path">配置ID: {p.packageId}<br />モデル保存領域: {p.modelsRoot}</p>
        <p>確認したモデル本体を削除します。アプリ内では復元できません。会議音声・本文・議事録、モデル名とSHA、構成履歴・ジョブ履歴・現在の設定は保持します。本体を失った構成は再準備まで利用できません。</p><p>他モデル、外部の導入元、バックアップは削除しません。媒体上の安全消去は保証しません。</p>
        <dl className="import-info"><dt>確認時のファイル・容量</dt><dd>{p.fileCount} 件 · {bytesLabel(p.bytes)}</dd><dt>欠落・登録時と変更・未登録</dt><dd>{p.missingCount} / {p.changedCount} / {p.unregisteredCount} パス（フォルダーを含む）</dd><dt>保持する構成・ジョブ参照</dt><dd>{p.profileReferences} / {p.jobReferences} 件</dd><dt>確認段階</dt><dd>{modelDeletionPhaseLabel(p.phase)}</dd><dt>処理済みパス</dt><dd>{p.processedPaths} / {p.pathCount}（フォルダー・欠落を含み、消したファイル数ではありません）</dd></dl>
        {p.pending && <p role="alert">途中まで本体を削除している可能性があります。既に消した本体は復元しません。再検査は新しい未知物を自動採用しません。</p>}
        {p.includeUnregisteredRecorded !== null && <p>保存済みの方針: 未登録パスを{p.includeUnregisteredRecorded ? "含める" : "含めない"}。この範囲の続行で方針を変更することはできません。</p>}
        {p.phase === "reconfirmation" && <p role="alert">以前の計画は {p.previousProcessedPaths} / {p.previousPathCount} パスを処理済みです。元の同意・進捗は履歴に残し、現在の新しい範囲へ再同意します。確認IDが変わると同意をやり直します。{p.priorDeletionCompleted && "元の本体削除は完了済みです。再出現した残りを確認しています。"}</p>}
        {!!p.reservationRoots.length && <p>元の計画から導いた予約ルートとモデル配置ルートを照合しています。下の一覧にある対象だけを扱い、隣のモデルや無関係の予約領域は対象にしません。</p>}
        {p.resumeReason && <p role="alert">削除を進められません: {p.resumeReason}</p>}
        {p.reconfirmationAllowed && <button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => void c.submit("review_remaining")}>モデル本体の変更後の残りを確認（まだ削除しない）</button>}
        <h3>確認時の対象パス（現在の実在一覧ではありません）</h3>{c.pageBusy && <p role="status">モデル対象一覧を読み込み中です。</p>}{c.pageError && <p role="alert">一覧を確認できません: {c.pageError}</p>}
        {c.page && <><p>{c.page.total ? c.page.offset + 1 : 0}〜{c.page.offset + c.page.entries.length} / {c.page.total} 件</p><ul className="deletion-paths">{c.page.entries.map((e, i) => <li key={e.relativePath}><span className="data-path">{e.relativePath}</span> — {e.directory ? "フォルダー" : !e.present ? "確認時に欠落" : bytesLabel(e.bytes!)} · {e.registered ? "登録済み" : "未登録"}{e.changed && "・登録時と変更"}{modelDeletionPathProcessed(p, c.page!.offset + i) && " · 処理済み"}</li>)}</ul></>}
        <div className="analysis-actions"><IconButton label="モデル対象の前の50件" icon="previous" type="button" className="small-secondary" disabled={c.pageBusy || !c.page || !c.page.offset} onClick={() => void c.loadPage(Math.max(0, (c.page?.offset ?? 0) - 50))} /><IconButton label="モデル対象の次の50件" icon="next" type="button" className="small-secondary" disabled={c.pageBusy || c.page?.nextOffset == null} onClick={() => void c.loadPage(c.page!.nextOffset!)} />{c.pageError && <IconButton label="モデル対象一覧を再読込" icon="refresh" type="button" className="small-secondary" onClick={() => void c.loadPage()} />}</div>
        {modelDeletionNewScope(p) && p.unregisteredCount > 0 && <label><input type="checkbox" checked={c.includeUnknown} disabled={!c.canResolve} onChange={e => c.setIncludeUnknown(e.target.checked)} />確認時の一覧にある未登録パスもモデル本体の削除対象に含めます。</label>}
        <label><input type="checkbox" checked={c.understood} disabled={!c.canResolve || !p.executeAllowed} onChange={e => c.setUnderstood(e.target.checked)} />対象を確認し、モデル本体はアプリ内では復元できず、会議データと履歴は保持されることを理解しました。</label>
      </>}
      <div className="analysis-actions"><button type="button" className="stop-button" disabled={!c.canDelete} onClick={() => void c.submit("publish")}>{p?.phase === "reconfirmation" ? "新しいモデル範囲へ再同意して削除を続行" : p?.pending ? "照合してモデル本体削除を続行" : "モデル本体を削除する"}</button><button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => void c.submit("refresh")}>モデル本体の残りを再検査</button>{p?.keepAllowed && p.pending && <button type="button" className="secondary" disabled={!c.canResolve} onClick={() => void c.submit("keep")}>モデル本体を未開始のまま保持して中止</button>}<button type="button" className="small-secondary" disabled={c.protectedNow || !!s?.busy} data-dialog-cancel onClick={() => void c.dismiss()}>{p?.pending ? "閉じる（モデル削除の未完了記録は残る）" : "モデル本体を削除せず閉じる"}</button></div>
      {c.unknown && <div className="analysis-actions"><button type="button" disabled={c.rpcBusy || !!s?.busy} onClick={() => void c.retryUnknown()}>同じモデル削除操作の結果を確認</button><button type="button" className="small-secondary" disabled={c.rpcBusy || !s?.token} onClick={() => void c.cancel()}>同じモデル操作の安全な停止を要求</button></div>}
      {(c.error || (active && s.errorMessage)) && <p className="action-error" role="alert">{c.error || s?.errorMessage}</p>}
    </CommonModal>
  </>;
}
