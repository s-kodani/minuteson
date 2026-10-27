import { DirectoryDeletionConfirmation } from "./directory-deletion-confirmation";
import {useDeletionNoticeWait} from "./deletion-notice-wait";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { rowMenuReturnTarget } from "./row-menu";
import { IssueNotice, ToastNotice } from "./toast";
import { CommonModal } from "./common-modal";
import { useEffect, useRef, useState } from "react";
import type { DataOperation } from "./data-operation";
import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state";
import { type DeletionReceipt } from "./deletion-state";
import { stopMeetingPlayback } from "./playback-barrier";
import { useMeetingDeletionAvailability } from "./meeting-deletion-availability";
import { deletionReviewReason } from "./deletion-review-reason";

export function useMeetingDeletion({ operation, locked, withSavedEdits, onDeleted, meetingIds }: {
  meetingIds: string[];
  operation: DataOperation; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void | "cancelled">;
  onDeleted: (receipt: DeletionReceipt) => Promise<void>;
}) {
  const noticeWaiting=useDeletionNoticeWait();
  const latest = useRef({ operation, locked, withSavedEdits, onDeleted }); latest.current = { operation, locked, withSavedEdits, onDeleted };
  const availability = useMeetingDeletionAvailability(meetingIds);
  const [rpcBusy, setRpcBusy] = useState(false), running = useRef(false);
  const [unknown, setUnknown] = useState(false), [open, setOpen] = useState(false);
  const [error, setError] = useState(""), [toast, setToast] = useState("");
  const [toastEvent, setToastEvent] = useState("");
  const pending = useRef<{ request: DataRequest; revision: number } | null>(null);
  const choice = useRef<{ id: string; pending: boolean; revision: number } | null>(null);
  const completed = useRef<string | null>(null), opened = useRef<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null), heading = useRef<HTMLHeadingElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const s = operation.snapshot, preview = s?.kind === "deletion" ? s.deletionPreview : null;
  const receipt = s?.kind === "deletion" ? s.deletionReceipt : null;
  const listing = operation.deletionListing;
  const protectedNow = rpcBusy || unknown;
  const canInspect = isTauri() && operation.deletionSupported && operation.connected && !locked && !rpcBusy && !unknown && !s?.busy;
  // Restart normally forbids all work. A canonical pending deletion alone can
  // use its restricted native resume lane; it never enables ordinary writes.
  const sharedAvailable = !!operation.availability?.available || (!!listing?.restartRequired && operation.availability?.reason === "保存先の切替後は終了して再起動が必要です");
  const canReviewPending = canInspect && (sharedAvailable || (!listing?.restartRequired && !s?.restartRequired));
  const canReviewMeeting = canInspect && !listing?.writeBlocked && !operation.modelDeletionListing?.writeBlocked && !operation.audioDeletionListing?.writeBlocked && !s?.audioDeletionPreview?.pending && !s?.restartRequired && !preview?.pending;
  const canReviewMeetingId = (id: string) => canReviewMeeting && availability.rows.get(id)?.available === true;
  const reviewUnavailableReason = canReviewMeeting ? null : deletionReviewReason(operation, { native: isTauri(), supported: operation.deletionSupported, locked, protectedNow, sharedAvailable });
  const meetingReviewReason = (id: string) => !canReviewMeeting ? reviewUnavailableReason : availability.error || availability.rows.get(id)?.blockers.map(b => b.reason).join(" ") || (availability.rows.has(id) ? null : "削除できる状態か確認しています。");
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
  const review = async (id: string, pendingReview: boolean) => {
    if (!(pendingReview ? canReviewPending || (canInspect && !listing?.restartRequired) : canReviewMeetingId(id))) return;
    const active = rowMenuReturnTarget(document.activeElement instanceof HTMLElement ? document.activeElement : null);
    await protect(async () => {
      await stopMeetingPlayback(pendingReview ? preview?.meetingId ?? listing?.items.find(item => item.operationId === id)?.meetingId ?? "" : id); opener.current = active; setOpen(false); setToast("");
      choice.current = { id, pending: pendingReview, revision: latest.current.operation.getSnapshot()?.revision ?? -1 };
      try {
        latest.current.operation.accept(await invoke<DataSnapshot>(pendingReview ? "review_pending_deletion" : "review_meeting_deletion", pendingReview ? { operationId: id } : { meetingId: id }));
        choice.current = null; setUnknown(false);
      } catch (cause) {
        if (typeof cause === "string" && cause.startsWith("meeting_deletion_not_started:")) { choice.current = null; setUnknown(false); void availability.refresh(); throw Error(cause.slice("meeting_deletion_not_started:".length)); }
        setUnknown(true); throw Error(`確認の受付を確定できません。削除は実行せず、同じ確認の状態を読み直してください: ${String(cause)}`);
      }
    });
  };
  const send = async (request: DataRequest, revision: number) => {
    pending.current = { request, revision };
    try { latest.current.operation.accept(await invoke<DataSnapshot>("start_data_operation", { request })); pending.current = null; setUnknown(false); setOpen(false); }
    catch (cause) {
      if (typeof cause === "string" && cause.startsWith("meeting_deletion_not_started:")) { pending.current = null; setUnknown(false); void availability.refresh(); throw Error(cause.slice("meeting_deletion_not_started:".length)); }
      setUnknown(true); throw Error(`受付結果を確定できません。別の削除を始めず、同じ操作の結果を確認してください: ${String(cause)}`);
    }
  };
  const submit = async (action: DataRequest["action"]) => {
    if (!canResolve || (preview?.scopeMode === "directory" && action === "publish" && !open)) return;
    const confirmation = preview?.confirmationId;
    if (!confirmation) return;
    if (action === "publish" && (preview?.scopeMode !== "directory" || !open || s?.phase !== "ready" || !preview.executeAllowed)) return;
    await protect(async () => {
      await stopMeetingPlayback(preview?.meetingId ?? "");
      const now = latest.current.operation.getSnapshot(); if (!now) throw Error("削除状態を再確認してください。");
      if (now.deletionPreview?.confirmationId !== confirmation) throw Error("待機中に削除の確認が変更されました。新しい一覧を確認してください。");
      await send(dataRequest(now, action), now.revision);
    });
  };
  const retryUnknown = async () => {
    if (running.current || locked || s?.busy) return;
    await protect(async () => {
      if (pending.current) { const p = pending.current; await stopMeetingPlayback(preview?.meetingId ?? ""); await send(p.request, p.revision); }
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
      setToast(receipt.deleted ? "会議の管理データを削除しました。この操作は取り消せません。" : "ファイル削除を開始せず保持して中止しました。部分削除の復元ではありません。");
      if (receipt.deleted) void latest.current.onDeleted(receipt).catch(cause => setError(`削除は完了しています。会議一覧を再読込してください: ${String(cause)}`));
      void latest.current.operation.refresh().catch(cause => setError(String(cause)));
    }
  }, [s, rpcBusy, locked, unknown]);
  const targetUnavailableReason = preview && !preview.pending ? availability.error || availability.rows.get(preview.meetingId)?.blockers.map(b => b.reason).join(" ") || (!availability.rows.has(preview.meetingId) ? "削除できる状態か確認しています。" : null) : null;
  const canDelete = preview?.scopeMode === "directory" && canResolve && open && s?.phase === "ready" && !!preview.executeAllowed && !targetUnavailableReason;
  return { reviewMeeting: (id: string) => review(id, false), reviewPending: (id: string) => review(id, true), canReviewMeeting, canReviewPending, canResolve,
    canReviewMeetingId, meetingReviewReason, targetUnavailableReason, meetingBlockers: (id: string) => availability.rows.get(id)?.blockers ?? [], retryAvailability: availability.refresh,
    noticeWaiting, reviewUnavailableReason, rpcBusy, unknown, protectedNow, open, setOpen, preview, receipt, listing, error, toast, toastEvent, setToast,
    canDelete, submit, retryUnknown, cancel, dismiss, quit, dialog, heading, opener };
}

export function MeetingDeletionPanel({ operation, control }: { operation: DataOperation; control: ReturnType<typeof useMeetingDeletion> }) {
  const c = control, s = operation.snapshot, p = c.preview;
  const active = s?.kind === "deletion";
  return <>
    <ToastNotice scope="deletion" type="meeting-result" eventId={c.toastEvent} message={c.toast} onClose={() => c.setToast("")} />
    <IssueNotice scope={p?.meetingId ?? "deletion"} type="meeting-deletion" label="会議削除" message={c.error || (active ? s?.errorMessage : "")} />
    {c.rpcBusy&&c.noticeWaiting&&<p role="status">通知履歴の処理完了を待っています</p>}
    {p?.scopeMode === "directory" && c.unknown && <div className="analysis-actions"><button type="button" disabled={c.rpcBusy || !!s?.busy} onClick={() => void c.retryUnknown()}>削除要求の結果を確認</button><button type="button" disabled={c.rpcBusy || !s?.token} onClick={() => void c.cancel()}>削除を停止</button></div>}
    <CommonModal portal hideDismissButton ref={c.dialog} open={c.open && p?.scopeMode === "directory"} blocked={c.protectedNow || !!s?.busy} onDismiss={c.dismiss} restoreReady={!c.rpcBusy && !s?.busy && operation.connected} restoreTarget={() => c.opener.current?.isConnected && !c.opener.current.matches(":disabled") ? c.opener.current : document.querySelector<HTMLElement>(".sidebar button")} className="import-dialog data-dialog deletion-dialog" aria-labelledby="deletion-confirm-title">
      {p?.scopeMode === "directory" && <DirectoryDeletionConfirmation kind="meeting" name={p.title} directory={p.targetDirectory!} titleId="deletion-confirm-title" heading={c.heading} blocked={c.protectedNow || !!s?.busy} canDelete={c.canDelete} onCancel={() => void c.dismiss()} onDelete={() => void c.submit("publish")} />}
    </CommonModal>
  </>;
}
