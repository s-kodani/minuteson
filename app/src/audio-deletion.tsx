import { IconButton } from "./action-icon";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { rowMenuReturnTarget } from "./row-menu";
import { IssueNotice, ToastNotice } from "./toast";
import { CommonModal } from "./common-modal";
import { useEffect, useRef, useState } from "react";
import type { DataOperation } from "./data-operation";
import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state";
import { assertAudioDeletionPage, type AudioDeletionPage, type AudioDeletionReceipt } from "./audio-deletion-operation-state";
import { audioDeletionDispositionLabel, audioDeletionNewScope, audioDeletionPhaseLabel, audioDeletionPublishRequest } from "./audio-deletion-ui-state";
import { bytesLabel } from "./import-state";
import { stopAllPlayback } from "./playback-barrier";

export function useAudioDeletion({ operation, locked, withSavedEdits, onDeleted, onProtectionChange }: {
  operation: DataOperation; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void | "cancelled">;
  onDeleted: (receipt: AudioDeletionReceipt) => Promise<void>;
  onProtectionChange: (protectedNow: boolean) => void;
}) {
  const latest = useRef({ operation, locked, withSavedEdits, onDeleted, onProtectionChange }); latest.current = { operation, locked, withSavedEdits, onDeleted, onProtectionChange };
  const running = useRef(false), [rpcBusy, setRpcBusy] = useState(false);
  const [unknown, setUnknown] = useState(false), [open, setOpen] = useState(false);
  const [error, setError] = useState(""), [toast, setToast] = useState("");
  const [toastEvent, setToastEvent] = useState("");
  const [page, setPage] = useState<AudioDeletionPage | null>(null), [pageError, setPageError] = useState(""), [pageBusy, setPageBusy] = useState(false);
  const pageEpoch = useRef(0), [understoodId, setUnderstoodId] = useState<string | null>(null), [includeUnknownId, setIncludeUnknownId] = useState<string | null>(null);
  const pending = useRef<{ request: DataRequest; revision: number } | null>(null);
  const choice = useRef<{ id: string; pending: boolean; revision: number } | null>(null);
  const completed = useRef<string | null>(null), opened = useRef<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null), heading = useRef<HTMLHeadingElement>(null), opener = useRef<HTMLElement | null>(null);
  const s = operation.snapshot, preview = s?.kind === "audio_deletion" ? s.audioDeletionPreview : null, receipt = s?.kind === "audio_deletion" ? s.audioDeletionReceipt : null;
  const listing = operation.audioDeletionListing, protectedNow = rpcBusy || unknown;
  const sharedAvailable = !!operation.availability?.available || (!!listing?.restartRequired && operation.availability?.reason === "保存先の切替後は終了して再起動が必要です");
  const canReviewPending = isTauri() && operation.audioDeletionSupported && operation.connected && sharedAvailable && !locked && !protectedNow && !s?.busy;
  const canReviewNew = canReviewPending && !listing?.writeBlocked && !operation.deletionListing?.writeBlocked && !operation.modelDeletionListing?.writeBlocked && !s?.restartRequired && !preview?.pending && !s?.deletionPreview?.pending && !s?.modelDeletionPreview?.pending;
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
    if (op.audioDeletionListing?.writeBlocked || op.deletionListing?.writeBlocked || op.modelDeletionListing?.writeBlocked || now?.audioDeletionPreview?.pending || now?.deletionPreview?.pending || now?.modelDeletionPreview?.pending) await run(action);
    else await latest.current.withSavedEdits(async () => run(action));
  };
  const loadPage = async (offset = 0) => {
    const now = latest.current.operation.getSnapshot(), p = now?.kind === "audio_deletion" ? now.audioDeletionPreview : null;
    if (!p || now?.busy) return;
    const epoch = ++pageEpoch.current; setPage(null); setPageError(""); setPageBusy(true);
    try {
      const value = await invoke<AudioDeletionPage>("get_audio_deletion_paths", { confirmationId: p.confirmationId, offset });
      assertAudioDeletionPage(value, p, offset);
      if (epoch === pageEpoch.current && latest.current.operation.getSnapshot()?.audioDeletionPreview?.confirmationId === p.confirmationId) setPage(value);
    } catch (cause) { if (epoch === pageEpoch.current) setPageError(String(cause)); }
    finally { if (epoch === pageEpoch.current) setPageBusy(false); }
  };
  const review = async (id: string, pendingReview: boolean) => {
    if (!(pendingReview ? canReviewPending : canReviewNew)) return;
    const active = rowMenuReturnTarget(document.activeElement instanceof HTMLElement ? document.activeElement : null);
    await protect(async () => {
      await stopAllPlayback(); opener.current = active; setOpen(false); setToast("");
      choice.current = { id, pending: pendingReview, revision: latest.current.operation.getSnapshot()?.revision ?? -1 };
      try {
        latest.current.operation.accept(await invoke<DataSnapshot>(pendingReview ? "review_pending_audio_deletion" : "review_meeting_audio_deletion", pendingReview ? { operationId: id } : { meetingId: id }));
        choice.current = null; setUnknown(false);
      } catch (cause) { setUnknown(true); throw Error(`確認の受付を確定できません。削除は実行せず、同じ確認の状態を読み直してください: ${String(cause)}`); }
    });
  };
  const send = async (request: DataRequest, revision: number) => {
    pending.current = { request, revision };
    try { latest.current.operation.accept(await invoke<DataSnapshot>("start_data_operation", { request })); pending.current = null; setUnknown(false); setOpen(false); }
    catch (cause) { setUnknown(true); throw Error(`受付結果を確定できません。別の削除を始めず、同じ音声削除操作の結果を確認してください: ${String(cause)}`); }
  };
  const submit = async (action: DataRequest["action"]) => {
    if (!canResolve || !preview) return;
    const confirmation = preview.confirmationId, token = s?.token, publishing = action === "publish";
    if (publishing && (!open || !understood || !page || pageBusy || pageError)) return;
    await protect(async () => {
      await stopAllPlayback();
      const now = latest.current.operation.getSnapshot();
      if (!now || now.kind !== "audio_deletion" || now.token !== token || now.audioDeletionPreview?.confirmationId !== confirmation || !latest.current.operation.connected) throw Error("待機中に音声削除の確認が変更されました。新しい状態を確認してください。");
      const request = publishing ? audioDeletionPublishRequest(now, confirmation, page, understoodId, includeUnknownId) : dataRequest(now, action);
      await send(request, now.revision);
    });
  };
  const matchesChoice = (now: DataSnapshot, c: NonNullable<typeof choice.current>) => now.kind === "audio_deletion" && now.revision > c.revision && (c.pending ? now.audioDeletionPreview?.operationId === c.id : now.audioDeletionPreview?.meetingId === c.id);
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
    if (running.current || now?.kind !== "audio_deletion" || !now.token) return;
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
    if (!s || s.kind !== "audio_deletion") { setOpen(false); return; }
    if (s.busy || receipt) setOpen(false);
    if (unknown) {
      const c = choice.current, p = pending.current;
      if ((c && matchesChoice(s, c)) || (p && s.token === p.request.token && s.revision > p.revision && (s.busy || ["completed", "failed", "cancelled", "attention"].includes(s.phase) || (s.phase === "ready" && preview?.confirmationId !== p.request.confirmationId)))) { pending.current = null; choice.current = null; setUnknown(false); setError(""); }
    }
    if (s.phase === "ready" && preview && !rpcBusy && !locked && !unknown && opened.current !== preview.confirmationId) { opened.current = preview.confirmationId; setOpen(true); }
    if (receipt && completed.current !== s.token) {
      completed.current = s.token; setToastEvent(s.token ?? ""); setError("");
      setToast(receipt.audioDeleted ? "音声だけを削除しました。文字起こし・議事録・根拠テキストと履歴は保持しています。音声の再生・解析・再解析はできません。" : "音声削除を開始せず保持して中止しました。部分削除の復元ではありません。");
      if (receipt.audioDeleted) void latest.current.onDeleted(receipt).catch(cause => setError(`音声削除は完了しています。会議一覧を再読込してください: ${String(cause)}`));
      void latest.current.operation.refresh().catch(cause => setError(String(cause)));
    }
  }, [s, rpcBusy, locked, unknown]);
  let canDelete = false;
  if (canResolve && open && understood && page && !pageBusy && !pageError && s && preview) {
    try { audioDeletionPublishRequest(s, preview.confirmationId, page, understoodId, includeUnknownId); canDelete = true; } catch { /* Fail closed on stale consent/page/phase. */ }
  }
  return { reviewNew: (id: string) => review(id, false), reviewPending: (id: string) => review(id, true), canReviewNew, canReviewPending, canResolve,
    protectedNow, rpcBusy, unknown, open, setOpen, error, toast, toastEvent, setToast, preview, receipt, listing, page, pageBusy, pageError, loadPage,
    understood, setUnderstood: (v: boolean) => setUnderstoodId(v ? preview?.confirmationId ?? null : null), includeUnknown, setIncludeUnknown: (v: boolean) => setIncludeUnknownId(v ? preview?.confirmationId ?? null : null),
    canDelete, submit, cancel, dismiss, retryUnknown, quit, dialog, heading, opener };
}
export type AudioDeletionControl = ReturnType<typeof useAudioDeletion>;

export function AudioDeletionPanel({ operation, control: c, meetingTitle }: { operation: DataOperation; control: AudioDeletionControl; meetingTitle?: string }) {
  const s = operation.snapshot, p = c.preview, active = s?.kind === "audio_deletion";
  const trackKind = (kind: string) => ({ mic: "マイク", system: "システム音声", imported: "取り込み音声", processed: "処理済み音声" } as Record<string, string>)[kind] ?? kind;
  const trackState = (state: string) => ({ ready: "保存済み", interrupted: "中断", failed: "失敗" } as Record<string, string>)[state] ?? state;
  return <>
    <ToastNotice scope="audio" type="deletion-result" eventId={c.toastEvent} message={c.toast} onClose={() => c.setToast("")} />
    <IssueNotice scope={p?.meetingId ?? "deletion"} type="audio-deletion" label="音声削除" message={c.error || (active ? s?.errorMessage : "")} />
    <CommonModal portal ref={c.dialog} open={c.open} blocked={c.protectedNow || !!s?.busy} onDismiss={c.dismiss} restoreReady={!c.rpcBusy && !s?.busy && operation.connected} restoreTarget={() => c.opener.current?.isConnected && !c.opener.current.matches(":disabled") ? c.opener.current : document.querySelector<HTMLElement>(".sidebar button")} className="import-dialog data-dialog deletion-dialog" aria-labelledby="audio-deletion-title">
      <h2 id="audio-deletion-title" ref={c.heading} tabIndex={-1}>{p?.phase === "reconfirmation" ? p.reconfirmationSourceKind === "native_audio_pending" ? "残り音声への新しい同意" : "再出現した音声への新しい同意" : "音声だけの削除を確認"}</h2>
      {p && <><h3 className="data-path">会議名（一覧表示）: {meetingTitle ?? "確認できません。会議IDを照合してください。"}</h3><p className="data-path">会議ID: {p.meetingId}<br />操作ID: {p.operationId}</p>
        <p>確認した管理音声だけを削除します。アプリ内では復元できません。音声の再生・解析・再解析と、根拠からの音声再生ができなくなります。文字起こし・議事録・下書き・根拠テキストと履歴は保持します。残ったテキストからの議事録生成は音声解析とは別です。</p><p>外部の原本・出力済み文書・バックアップは削除しません。媒体上の安全消去は保証しません。</p>
        <dl className="import-info"><dt>確認時の音声ファイル・容量</dt><dd>{p.audioFiles} 件 · {bytesLabel(p.audioBytes)}</dd><dt>欠落・登録時と変更・未登録音声</dt><dd>{p.missingAudioFiles} / {p.changedAudioFiles} / {p.unregisteredAudioFiles} 件</dd><dt>分類できないファイル（必ず保持）</dt><dd>{p.unclassifiedFiles} 件</dd><dt>保持する解析版・発言・議事録版</dt><dd>{p.retained.analysisRevisions} / {p.retained.utterances} / {p.retained.minutesRevisions} 件</dd><dt>保持する下書き・編集履歴・根拠</dt><dd>{p.retained.drafts} / {p.retained.editOperations} / {p.retained.evidenceLinks} 件</dd><dt>保持するジョブ・外部出力記録</dt><dd>{p.retained.jobs} / {p.retained.externalExportRecords} 件</dd><dt>確認段階</dt><dd>{audioDeletionPhaseLabel(p.phase)}</dd><dt>処理済みの選択音声</dt><dd>{p.processedFiles} 件（欠落を含み、実際に消したファイル数ではありません。下の全パス一覧の位置とは対応しません）</dd></dl>
        <h3>確認時の音声トラック</h3>{p.tracks.length ? <ul>{p.tracks.map(t => <li key={t.id}>{trackKind(t.kind)} · {trackState(t.state)} · {t.chunks} 区間 · {(t.durationUs / 1_000_000).toLocaleString("ja-JP")} 秒 <span className="data-path">{t.id}</span></li>)}</ul> : <p>登録されたトラックはありません。ファイル一覧を確認してください。</p>}
        {p.phase === "restored_unavailable" && <p role="alert">元の操作 {p.operationId} は復元時の音声利用不可記録です。音声を削除した証拠ではなく、削除同意もありません。この記録は変更せず保持します。現在の対象を確認し、新しい操作で別同意するまで音声を削除しません。</p>}
        {p.pending && !["restored_unavailable", "reconfirmation"].includes(p.phase) && <p role="alert">既に一部の音声を削除している可能性があります。消した音声は復元しません。再検査だけで新しい範囲へ同意した扱いにはしません。</p>}
        {p.includeUnregisteredRecorded !== null && <p>保存済みの方針: 未登録音声を{p.includeUnregisteredRecorded ? "含める" : "含めない"}。続行で方針を変更できません。分類できないファイルは保持します。</p>}
        {p.phase === "reconfirmation" && <p role="alert">{p.reconfirmationSourceKind === "native_audio_pending" ? <>操作 {p.operationId} の音声削除は未完了です。既に消した音声は復元しません。元の同意・対象証拠・中断位置を履歴として保持し、同じ操作の現在の残り範囲への新しい同意を別記録に保存して続行します。確認だけでは同意を保存せず、音声も削除しません。</> : <>{p.reconfirmationSourceKind === "restored_audio_unavailable" ? <>元の操作 {p.sourceOperationId} は復元時の音声利用不可記録で、削除同意はありません。元の記録と復元履歴を変更せず、</> : <>元の操作 {p.sourceOperationId} は音声削除完了済みです。元の同意と履歴を変更せず、</>}新しい操作 {p.operationId} で現在の範囲へ別同意します。この新しい操作ではまだ音声削除を開始していません。</>}確認IDが変わると同意をやり直します。</p>}
        {p.sourceOperationId && p.phase !== "reconfirmation" && <p className="data-path">参照元の記録ID: {p.sourceOperationId}（元の記録は変更しません）</p>}
        {p.resumeReason && <p role="alert">削除を進められません: {p.resumeReason}。対象と未完了記録は保持します。</p>}
        {p.reconfirmationAllowed && <button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => void c.submit("review_remaining")}>{["files", "availability"].includes(p.phase) ? "残り音声を確認（まだ削除しない）" : "再出現した音声を確認（まだ削除しない）"}</button>}
        <h3>確認時のファイル一覧（現在の実在一覧ではありません）</h3>{c.pageBusy && <p role="status">音声対象一覧を読み込み中です。</p>}{c.pageError && <p role="alert">一覧を確認できません: {c.pageError}</p>}
        {c.page && <><p>{c.page.total ? c.page.offset + 1 : 0}〜{c.page.offset + c.page.entries.length} / {c.page.total} 件</p><ul className="deletion-paths">{c.page.entries.map(e => <li key={e.relativePath}><span className="data-path">{e.relativePath}</span> — {audioDeletionDispositionLabel(e.disposition)} · {!e.present ? "確認時に欠落" : bytesLabel(e.bytes!)}{e.changed && "・登録時と変更"}</li>)}</ul></>}
        <div className="analysis-actions"><IconButton label="音声対象の前の50件" icon="previous" type="button" className="small-secondary" disabled={c.pageBusy || !c.page || !c.page.offset} onClick={() => void c.loadPage(Math.max(0, (c.page?.offset ?? 0) - 50))} /><IconButton label="音声対象の次の50件" icon="next" type="button" className="small-secondary" disabled={c.pageBusy || c.page?.nextOffset == null} onClick={() => void c.loadPage(c.page!.nextOffset!)} />{c.pageError && <IconButton label="音声対象一覧を再読込" icon="refresh" type="button" className="small-secondary" onClick={() => void c.loadPage()} />}</div>
        {audioDeletionNewScope(p) && p.unregisteredAudioFiles > 0 && <label><input type="checkbox" checked={c.includeUnknown} disabled={!c.canResolve} onChange={e => c.setIncludeUnknown(e.target.checked)} />一覧にある未登録音声も削除対象に含めます。分類できないファイルは保持します。</label>}
        <label><input type="checkbox" checked={c.understood} disabled={!c.canResolve || !p.executeAllowed} onChange={e => c.setUnderstood(e.target.checked)} />対象を確認し、音声の再生・解析・再解析ができなくなり、音声は復元できず、文書と履歴は保持されることを理解しました。</label>
      </>}
      <div className="analysis-actions"><button type="button" className="stop-button" disabled={!c.canDelete} onClick={() => void c.submit("publish")}>{p?.phase === "restored_unavailable" ? "元の利用不可記録からは削除できません" : p?.phase === "reconfirmation" ? "新しい音声範囲へ別同意して削除" : p?.pending ? "照合して音声削除を続行" : "音声だけを削除する"}</button><button type="button" className="small-secondary" disabled={!c.canResolve} onClick={() => void c.submit("refresh")}>音声の残りを再検査</button>{p?.keepAllowed && p.pending && <button type="button" className="secondary" disabled={!c.canResolve} onClick={() => void c.submit("keep")}>音声を未開始のまま保持して中止</button>}<button type="button" className="small-secondary" disabled={c.protectedNow || !!s?.busy} data-dialog-cancel onClick={() => void c.dismiss()}>{p?.phase === "restored_unavailable" ? "閉じる（元の利用不可記録は残る）" : p?.pending ? "閉じる（音声削除の未完了記録は残る）" : "音声を削除せず閉じる"}</button></div>
      {c.unknown && <div className="analysis-actions"><button type="button" disabled={c.rpcBusy || !!s?.busy} onClick={() => void c.retryUnknown()}>同じ音声削除操作の結果を確認</button><button type="button" className="small-secondary" disabled={c.rpcBusy || !s?.token} onClick={() => void c.cancel()}>同じ音声操作の安全な停止を要求</button></div>}
      {(c.error || (active && s.errorMessage)) && <p className="action-error" role="alert">{c.error || s?.errorMessage}</p>}
    </CommonModal>
  </>;
}
