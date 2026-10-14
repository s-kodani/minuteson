import { IconButton } from "./action-icon";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { rowMenuReturnTarget } from "./row-menu";
import { IssueNotice, ToastNotice } from "./toast";
import { CommonModal } from "./common-modal";
import { useEffect, useRef, useState } from "react";
import type { DataOperation } from "./data-operation";
import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state";
import { assertDeletionPathPage, deletionPhaseLabel, isDeletionReconfirmation, type DeletionPathPage, type DeletionReceipt } from "./deletion-state";
import { bytesLabel } from "./import-state";
import { stopAllPlayback } from "./playback-barrier";

export function useMeetingDeletion({ operation, locked, withSavedEdits, onDeleted }: {
  operation: DataOperation; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void | "cancelled">;
  onDeleted: (receipt: DeletionReceipt) => Promise<void>;
}) {
  const latest = useRef({ operation, locked, withSavedEdits, onDeleted }); latest.current = { operation, locked, withSavedEdits, onDeleted };
  const [rpcBusy, setRpcBusy] = useState(false), running = useRef(false);
  const [unknown, setUnknown] = useState(false), [open, setOpen] = useState(false);
  const [error, setError] = useState(""), [toast, setToast] = useState("");
  const [toastEvent, setToastEvent] = useState("");
  const [page, setPage] = useState<DeletionPathPage | null>(null), [pageError, setPageError] = useState("");
  const [pageBusy, setPageBusy] = useState(false), pageEpoch = useRef(0);
  const [understood, saveUnderstood] = useState(false), [includeUnknown, saveIncludeUnknown] = useState(false);
  const [understoodId, setUnderstoodId] = useState<string | null>(null), [includeUnknownId, setIncludeUnknownId] = useState<string | null>(null);
  const pending = useRef<{ request: DataRequest; revision: number } | null>(null);
  const choice = useRef<{ id: string; pending: boolean; revision: number } | null>(null);
  const completed = useRef<string | null>(null), opened = useRef<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null), heading = useRef<HTMLHeadingElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const s = operation.snapshot, preview = s?.kind === "deletion" ? s.deletionPreview : null;
  const receipt = s?.kind === "deletion" ? s.deletionReceipt : null;
  const currentUnderstood = understood && understoodId === preview?.confirmationId;
  const currentIncludeUnknown = includeUnknown && includeUnknownId === preview?.confirmationId;
  const setUnderstood = (value: boolean) => { saveUnderstood(value); setUnderstoodId(preview?.confirmationId ?? null); };
  const setIncludeUnknown = (value: boolean) => { saveIncludeUnknown(value); setIncludeUnknownId(preview?.confirmationId ?? null); };
  const listing = operation.deletionListing;
  const protectedNow = rpcBusy || unknown;
  const canInspect = isTauri() && operation.deletionSupported && operation.connected && !locked && !rpcBusy && !unknown && !s?.busy;
  // Restart normally forbids all work. A canonical pending deletion alone can
  // use its restricted native resume lane; it never enables ordinary writes.
  const sharedAvailable = !!operation.availability?.available || (!!listing?.restartRequired && operation.availability?.reason === "保存先の切替後は終了して再起動が必要です");
  const canReviewPending = canInspect && sharedAvailable;
  const canReviewMeeting = canReviewPending && !listing?.writeBlocked && !operation.modelDeletionListing?.writeBlocked && !operation.audioDeletionListing?.writeBlocked && !s?.audioDeletionPreview?.pending && !s?.restartRequired && !preview?.pending;
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
    if (latest.current.operation.deletionListing?.writeBlocked || latest.current.operation.modelDeletionListing?.writeBlocked || latest.current.operation.audioDeletionListing?.writeBlocked || latest.current.operation.getSnapshot()?.audioDeletionPreview?.pending || latest.current.operation.getSnapshot()?.deletionPreview?.pending) await run(action);
    else await latest.current.withSavedEdits(async () => run(action));
  };
  const loadPage = async (offset = 0) => {
    const p = latest.current.operation.getSnapshot()?.deletionPreview;
    if (!p || latest.current.operation.getSnapshot()?.busy) return;
    const epoch = ++pageEpoch.current;
    setPage(null); setPageError(""); setPageBusy(true);
    try {
      const value = await invoke<DeletionPathPage>("get_deletion_paths", { confirmationId: p.confirmationId, offset });
      assertDeletionPathPage(value, p.confirmationId, offset, p.pathCount, p.meetingId, p.reservationRoots, p.operationId);
      if (epoch === pageEpoch.current && latest.current.operation.getSnapshot()?.deletionPreview?.confirmationId === p.confirmationId) setPage(value);
    } catch (cause) { if (epoch === pageEpoch.current) setPageError(String(cause)); }
    finally { if (epoch === pageEpoch.current) setPageBusy(false); }
  };
  const review = async (id: string, pendingReview: boolean) => {
    if (!(pendingReview ? canReviewPending : canReviewMeeting)) return;
    const active = rowMenuReturnTarget(document.activeElement instanceof HTMLElement ? document.activeElement : null);
    await protect(async () => {
      await stopAllPlayback(); opener.current = active; setOpen(false); setToast("");
      choice.current = { id, pending: pendingReview, revision: latest.current.operation.getSnapshot()?.revision ?? -1 };
      try {
        latest.current.operation.accept(await invoke<DataSnapshot>(pendingReview ? "review_pending_deletion" : "review_meeting_deletion", pendingReview ? { operationId: id } : { meetingId: id }));
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
      if (now.deletionPreview?.confirmationId !== confirmation) throw Error("待機中に削除の確認が変更されました。新しい一覧を確認してください。");
      await send(dataRequest(now, action), now.revision);
    });
  };
  const retryUnknown = async () => {
    if (running.current || locked || s?.busy) return;
    await protect(async () => {
      if (pending.current) { const p = pending.current; await stopAllPlayback(); await send(p.request, p.revision); }
      else { await latest.current.operation.refresh(); const now = latest.current.operation.getSnapshot(), c = choice.current;
        if (c && now?.kind === "deletion" && now.revision > c.revision && (now.busy || ["failed", "cancelled", "attention"].includes(now.phase) || (c.pending ? now.deletionPreview?.operationId === c.id : now.deletionPreview?.meetingId === c.id))) { choice.current = null; setUnknown(false); }
        else throw Error("同じ確認の受付をまだ確定できません。読み直すか、安全な停止を要求してください。");
      }
    });
  };
  const cancel = async () => {
    const now = latest.current.operation.getSnapshot();
    if (running.current || now?.kind !== "deletion" || !now.token) return;
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
    if (!s || s.kind !== "deletion") { setOpen(false); return; }
    if (s.busy || receipt) setOpen(false);
    if (unknown) {
      const c = choice.current, p = pending.current;
      if ((c && s.revision > c.revision && (s.busy || ["failed", "cancelled", "attention"].includes(s.phase) || (c.pending ? preview?.operationId === c.id : preview?.meetingId === c.id)))
        || (p && s.token === p.request.token && s.revision > p.revision && (s.busy || ["completed", "failed", "cancelled", "attention"].includes(s.phase) || (s.phase === "ready" && preview?.confirmationId !== p.request.confirmationId)))) { pending.current = null; choice.current = null; setUnknown(false); }
    }
    if (s.phase === "ready" && preview && !rpcBusy && !locked && !unknown && opened.current !== preview.confirmationId) { opened.current = preview.confirmationId; setOpen(true); }
    if (receipt && completed.current !== s.token) {
      completed.current = s.token; setToastEvent(s.token ?? ""); setError("");
      setToast(receipt.deleted ? "会議の管理データを削除しました。アプリ内では復元できません。" : "ファイル削除を開始せず保持して中止しました。部分削除の復元ではありません。");
      if (receipt.deleted) void latest.current.onDeleted(receipt).catch(cause => setError(`削除は完了しています。会議一覧を再読込してください: ${String(cause)}`));
      void latest.current.operation.refresh().catch(cause => setError(String(cause)));
    }
  }, [s, rpcBusy, locked, unknown]);
  const canDelete = canResolve && s?.phase === "ready" && preview?.executeAllowed && currentUnderstood && !!page && page.confirmationId === preview.confirmationId && !pageBusy && !pageError && (!preview.unregisteredCount || !["review", "not_started", "reconfirmation", "database_reconfirmation"].includes(preview.phase) || currentIncludeUnknown);
  return { reviewMeeting: (id: string) => review(id, false), reviewPending: (id: string) => review(id, true), canReviewMeeting, canReviewPending, canResolve,
    rpcBusy, unknown, protectedNow, open, setOpen, preview, receipt, listing, error, toast, toastEvent, setToast, page, pageBusy, pageError, loadPage,
    understood: currentUnderstood, setUnderstood, includeUnknown: currentIncludeUnknown, setIncludeUnknown, canDelete, submit, retryUnknown, cancel, dismiss, quit, dialog, heading, opener };
}

export function MeetingDeletionPanel({ operation, control }: { operation: DataOperation; control: ReturnType<typeof useMeetingDeletion> }) {
  const c = control, s = operation.snapshot, p = c.preview;
  const active = s?.kind === "deletion";
  return <>
    <ToastNotice scope="deletion" type="meeting-result" eventId={c.toastEvent} message={c.toast} onClose={() => c.setToast("")} />
    <IssueNotice scope={p?.meetingId ?? "deletion"} type="meeting-deletion" label="会議削除" message={c.error || (active ? s?.errorMessage : "")} />
    <CommonModal portal ref={c.dialog} open={c.open} blocked={c.protectedNow || !!s?.busy} onDismiss={c.dismiss} restoreReady={!c.rpcBusy && !s?.busy && operation.connected} restoreTarget={() => c.opener.current?.isConnected && !c.opener.current.matches(":disabled") ? c.opener.current : document.querySelector<HTMLElement>(".sidebar button")} className="import-dialog data-dialog deletion-dialog" aria-labelledby="deletion-confirm-title">
      <h2 id="deletion-confirm-title" ref={c.heading} tabIndex={-1}>{isDeletionReconfirmation(p?.phase) ? "変更後の残り範囲への再同意" : "会議の完全削除を確認"}</h2>
      {p && <><h3>{p.title}</h3><p className="data-path">会議ID: {p.meetingId}</p><p>管理音声、文字起こし、議事録、現在・直前・内部参照版、候補、下書き・手動編集、会議別用語と処理履歴を削除します。アプリ内では復元できません。</p><p>外部の元音声・書き出し・バックアップ、AIモデル・共通用語、旧保存先・保持コピー・会議ツリー外の一時領域は対象外です。媒体上の安全消去は保証しません。</p><dl className="import-info"><dt>確認時のファイル</dt><dd>{p.fileCount} 件 · 実在容量 {bytesLabel(p.bytes)}</dd><dt>欠落・登録時と変更・未登録</dt><dd>{p.missingCount} / {p.changedCount} / {p.unregisteredCount} 件</dd><dt>{p.rowPhaseCommitted ? "元の確認時のDB行（削除済み）" : "確認時のDB行"}</dt><dd>{p.rowCount} 件</dd><dt>{isDeletionReconfirmation(p.phase) ? "現在の確認段階" : "保存済み段階"}</dt><dd>{deletionPhaseLabel(p.phase)}</dd><dt>処理済みパス</dt><dd>{p.processedPaths} / {p.pathCount}（フォルダー・欠落を含み、消したファイル数ではありません）</dd></dl>
      {p.pending && <p role="alert">途中まで削除している可能性があります。既に消したデータは復元しません。{isDeletionReconfirmation(p.phase) ? "この一覧は再同意前の確認です。再検査で確認IDが変わると、新しい範囲への同意をやり直します。" : "再検査は元の確認範囲を照合し、新しい未知物を自動採用しません。"}</p>}
      {isDeletionReconfirmation(p.phase) && <p role="alert">以前の計画では {p.previousProcessedPaths} / {p.previousPathCount} パスを処理済みです。ここには現在残る新しい範囲を表示しています。元の同意と進捗を履歴として保持し、再同意後に新しい計画で続行します。処理済みデータの復元や未開始への巻き戻しではありません。</p>}
      {p.reconfirmationAllowed && <button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => void c.submit("review_remaining")}>変更後の残り範囲を確認する（まだ削除しない）</button>}
      {p.rowPhaseCommitted && <p>会議のDB行は削除済みです。再作成せず、確認した残りのファイルと後処理・完了応答を扱います。</p>}
      {!!p.reservationRoots?.length && <p>元の削除計画から導いた会議ツリー外の予約ルートも照合しています。実在する予約対象は下の一覧に表示します。新しい会議フォルダーと併存する場合は両方を確認し、未登録ファイルには別の同意が必要です。隣の会議フォルダーは対象にしません。</p>}
      {p.resumeReason && <p role="alert">削除を進められません: {p.resumeReason}</p>}
      <h3>確認時の対象パス</h3>{c.pageBusy && <p role="status">一覧を読み込み中です。</p>}{c.pageError && <p role="alert">一覧を確認できません: {c.pageError}</p>}
      {c.page && <><p>{c.page.total ? c.page.offset + 1 : 0}〜{c.page.offset + c.page.entries.length} / {c.page.total} 件</p><ul className="deletion-paths">{c.page.entries.map(e => <li key={e.relativePath}><span className="data-path">{e.relativePath}</span> — {e.directory ? "フォルダー" : e.missing ? "欠落" : `${bytesLabel(e.bytes!)} · ${e.registered ? "登録済み" : "未登録"}${e.changed ? "・登録時と変更" : ""}`}{e.stageHint && ` · 由来候補 ${e.stageHint}（所有証明ではありません）`}</li>)}</ul></>}
      <div className="analysis-actions"><IconButton label="前の50件" icon="previous" type="button" className="small-secondary" disabled={c.pageBusy || !c.page || c.page.offset === 0} onClick={() => void c.loadPage(Math.max(0, (c.page?.offset ?? 0) - 50))} /><IconButton label="次の50件" icon="next" type="button" className="small-secondary" disabled={c.pageBusy || c.page?.nextOffset == null} onClick={() => void c.loadPage(c.page!.nextOffset!)} />{c.pageError && <IconButton label="一覧を再読込" icon="refresh" type="button" className="small-secondary" onClick={() => void c.loadPage()} />}</div>
      {p.unregisteredCount > 0 && ["review", "not_started", "reconfirmation", "database_reconfirmation"].includes(p.phase) && <label><input type="checkbox" checked={c.includeUnknown} disabled={!c.canResolve} onChange={event => c.setIncludeUnknown(event.target.checked)} />確認時の一覧にある未登録ファイルも削除対象に含める。由来候補は所有証明ではないことを確認しました。</label>}
      <label><input type="checkbox" checked={c.understood} disabled={!c.canResolve || !p.executeAllowed} onChange={event => c.setUnderstood(event.target.checked)} />対象と削除範囲を確認し、アプリ内では復元できないことを理解しました。</label>
      </>}
      <div className="analysis-actions"><button type="button" className="stop-button" disabled={!c.canDelete} onClick={() => void c.submit(c.includeUnknown && p && ["review", "not_started", "reconfirmation", "database_reconfirmation"].includes(p.phase) ? "delete_including_unregistered" : "publish")}>{isDeletionReconfirmation(p?.phase) ? "新しい範囲へ再同意して削除を続行" : p?.pending ? "照合して残りの削除を続行" : "会議を削除する"}</button><button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => void c.submit("refresh")}>残りの状態を再検査</button>{p?.keepAllowed && p.pending && <button type="button" className="secondary" disabled={!c.canResolve} onClick={() => void c.submit("keep")}>未開始のまま保持して中止</button>}<button type="button" className="small-secondary" disabled={c.rpcBusy || !!s?.busy || c.unknown} data-dialog-cancel onClick={() => void c.dismiss()}>{p?.pending ? "閉じる（未完了削除は残る）" : "削除せず閉じる"}</button></div>
      {c.unknown && <div className="analysis-actions"><button type="button" disabled={c.rpcBusy || !!s?.busy} onClick={() => void c.retryUnknown()}>同じ削除操作の結果を確認</button><button type="button" className="small-secondary" disabled={c.rpcBusy || !s?.token} onClick={() => void c.cancel()}>同じ操作の安全な停止を要求</button></div>}
      {(c.error || (active && s.errorMessage)) && <p className="action-error" role="alert">{c.error || s?.errorMessage}</p>}
    </CommonModal>
  </>;
}
