import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";
import { IssueNotice, ToastNotice } from "./toast";
import type { DataOperation } from "./data-operation";
import { dataRequest, dataPhaseLabel, type DataRequest, type DataSnapshot } from "./data-operation-state";
import { assertPartialRecordingListing, canRecoverPartialRecording, type PartialRecordingItem, type PartialRecordingListing } from "./partial-recording-state";
import { partialCancelObserved, partialChoiceObserved, partialRequestObserved, type PartialCancel, type PartialChoice, type PartialPending } from "./partial-recording-operation";
import { PartialRecordingSummary } from "./partial-recording-summary";
import { PartialRecordingResultsPanel } from "./partial-recording-results";
import { stopAllPlayback } from "./playback-barrier";
import { useActionConfirmation } from "./confirmation-service";
import { withConfirmedEdits } from "./confirmed-action";

export function PartialRecordingPanel({ operation, visible, locked, withSavedEdits, onProtectionChange, onRecovered }: {
  operation: DataOperation; visible: boolean; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  onProtectionChange: (active: boolean) => void;
  onRecovered: (meetingId: string) => Promise<void>;
}) {
  const latest = useRef({ operation, locked, withSavedEdits, onProtectionChange, onRecovered });
  latest.current = { operation, locked, withSavedEdits, onProtectionChange, onRecovered };
  const running = useRef(false), [rpcBusy, setRpcBusy] = useState(false);
  const [unknown, setUnknown] = useState(false), [open, setOpen] = useState(false), [error, setError] = useState("");
  const [savedActive, setSavedActive] = useState(false);
  const [understood, setUnderstood] = useState<string | null>(null);
  const pending = useRef<PartialPending | null>(null), choice = useRef<PartialChoice | null>(null);
  const stopping = useRef<PartialCancel | null>(null);
  const opened = useRef<string | null>(null), notified = useRef<string | null>(null);
  const [toast, setToast] = useState(""), [toastId, setToastId] = useState("");
  const [extra, setExtra] = useState<PartialRecordingListing | null>(null), [pageBusy, setPageBusy] = useState(false);
  const pageEpoch = useRef(0);
  const dialog = useRef<HTMLDialogElement>(null), opener = useRef<HTMLElement | null>(null);
  const confirm = useActionConfirmation(false);
  const s = operation.snapshot, active = s?.kind === "partial_recording";
  const preview = active ? s.partialRecordingPreview : null, receipt = active ? s.partialRecordingReceipt : null;
  const protectedNow = rpcBusy || unknown || confirm.pending || savedActive;
  const canAct = isTauri() && operation.partialRecordingSupported && operation.connected && !!operation.availability?.available &&
    !locked && !protectedNow && !s?.busy && !s?.restartRequired;
  const resolveUnknown = (now: DataSnapshot | null) => {
    if (stopping.current) {
      if (!partialCancelObserved(now, stopping.current)) return false;
      stopping.current = null; choice.current = null; pending.current = null; setUnknown(false); setOpen(false); return true;
    }
    if (choice.current && partialChoiceObserved(now, choice.current) || pending.current && partialRequestObserved(now, pending.current)) {
      choice.current = null; pending.current = null; setUnknown(false); return true;
    }
    return false;
  };
  const run = async (action: () => Promise<void>, propagate = false) => {
    if (running.current) { if (propagate) throw Error("別の部分救出操作が進行中です。"); return; }
    running.current = true; setRpcBusy(true); setError("");
    try { await action(); } catch (cause) { setError(String(cause)); if (propagate) throw cause; }
    finally { running.current = false; setRpcBusy(false); }
  };
  const protectedRun = async (action: () => Promise<void>, propagate = false) => {
    const work = async () => run(async () => { await stopAllPlayback(); await action(); }, propagate);
    if (propagate) await withConfirmedEdits(latest.current.withSavedEdits, work);
    else await latest.current.withSavedEdits(work);
  };
  const choose = async (item: PartialRecordingItem) => {
    if (!canAct) return;
    const expected = latest.current.operation.getSnapshot();
    const activeElement = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const same = () => latest.current.operation.connected && !!latest.current.operation.availability?.available &&
      latest.current.operation.getSnapshot()?.revision === expected?.revision && !latest.current.operation.getSnapshot()?.busy;
    const inspect = async (propagate = false) => protectedRun(async () => {
      if (!same()) throw Error("編集保存・再生停止の待機中に操作が変更されました。検査は開始していません。");
      opener.current = activeElement; setOpen(false);
      choice.current = { sessionId: item.sessionId, revision: expected?.revision ?? -1 };
      try {
        latest.current.operation.accept(await invoke<DataSnapshot>("review_partial_recording", { sessionId: item.sessionId }));
        choice.current = null; setUnknown(false);
      } catch (cause) {
        if (resolveUnknown(latest.current.operation.getSnapshot())) return;
        setUnknown(true);
        throw Error(`検査受付を確認できません。救出は実行せず、同じ操作の状態を確認してください: ${String(cause)}`);
      }
    }, propagate);
    if (expected?.retryAllowed) await confirm({ opener: activeElement, title: "部分救出の確認対象を変更", target: `${item.title} · ${item.sessionId}`,
      confirmLabel: "現在の確認を破棄して検査", message: "現在の確認内容と同意を破棄し、選んだ中断録音の確定範囲を検査します。音声は削除せず、検査だけでは救出しません。",
      validate: () => !latest.current.locked && !running.current && !unknown && same(), action: () => inspect(true) });
    else await inspect();
  };
  const send = async (request: DataRequest, revision: number) => {
    pending.current = { request, revision };
    try {
      latest.current.operation.accept(await invoke<DataSnapshot>("start_data_operation", { request }));
      pending.current = null; setUnknown(false); setOpen(false);
    } catch (cause) {
      if (resolveUnknown(latest.current.operation.getSnapshot())) return;
      setUnknown(true);
      throw Error(`受付結果を確認できません。別の救出は始めず、同じ操作の結果を確認してください: ${String(cause)}`);
    }
  };
  const submit = async (action: "recover" | "keep" | "refresh") => {
    if (!canAct || !preview || receipt || action === "recover" && (!open || understood !== preview.confirmationId)) return;
    const token = s?.token, confirmation = preview.confirmationId;
    await protectedRun(async () => {
      const now = latest.current.operation.getSnapshot();
      if (!latest.current.operation.connected || !now || now.kind !== "partial_recording" || now.token !== token ||
          now.partialRecordingPreview?.confirmationId !== confirmation) throw Error("部分救出の確認が待機中に変更されました。再確認してください。");
      await send(dataRequest(now, action), now.revision);
    });
  };
  const retryUnknown = async () => {
    if (running.current || locked || s?.busy) return;
    await protectedRun(async () => {
      await latest.current.operation.refresh();
      if (resolveUnknown(latest.current.operation.getSnapshot())) return;
      if (stopping.current) await sendCancel(stopping.current);
      else if (pending.current) await send(pending.current.request, pending.current.revision);
      else throw Error("検査受付はまだ確認できません。別の操作は開始せず、同じ状態を読み直してください。");
    });
  };
  const sendCancel = async (request: PartialCancel) => {
    stopping.current = request;
    try {
      latest.current.operation.accept(await invoke<DataSnapshot>("cancel_data_operation", { token: request.token }));
      stopping.current = null; pending.current = null; choice.current = null; setUnknown(false); setOpen(false);
    } catch (cause) {
      if (resolveUnknown(latest.current.operation.getSnapshot())) return;
      setUnknown(true);
      throw Error(`停止受付を確認できません。実際の終了まで保護を維持します: ${String(cause)}`);
    }
  };
  const cancel = async () => {
    const now = latest.current.operation.getSnapshot();
    if (running.current || now?.kind !== "partial_recording" || !now.token) return;
    await run(() => sendCancel({ token: now.token!, revision: now.revision }));
  };
  const loadMore = async () => {
    const after = extra ? extra.next : operation.partialRecordingListing?.next;
    if (!after || pageBusy) return;
    const epoch = ++pageEpoch.current; setPageBusy(true);
    try {
      const page = await invoke<PartialRecordingListing>("get_partial_recording_items", { after }); assertPartialRecordingListing(page, after);
      if (epoch === pageEpoch.current) setExtra(old => ({ items: [...(old?.items ?? []), ...page.items], next: page.next }));
    } catch (cause) { if (epoch === pageEpoch.current) setError(String(cause)); }
    finally { if (epoch === pageEpoch.current) setPageBusy(false); }
  };
  useEffect(() => { latest.current.onProtectionChange(protectedNow); return () => latest.current.onProtectionChange(false); }, [protectedNow]);
  const listIdentity = JSON.stringify(operation.partialRecordingListing);
  useEffect(() => { pageEpoch.current++; setExtra(null); setPageBusy(false); }, [listIdentity]);
  useEffect(() => { setUnderstood(null); }, [preview?.confirmationId]);
  useEffect(() => {
    if (!active) { setOpen(false); return; }
    if (s.busy || receipt) setOpen(false);
    if (unknown && resolveUnknown(s)) setError("");
    if (s.phase === "ready" && preview && !rpcBusy && !locked && !unknown && !confirm.pending && !savedActive && opened.current !== preview.confirmationId) {
      opened.current = preview.confirmationId; setOpen(true);
    }
    if (receipt && notified.current !== s.token) {
      notified.current = s.token; setToastId(s.token ?? "");
      setToast(receipt.recovered ? "検証済みの確定範囲だけを救出しました。取得終了と末尾損失は不明のままです。原音声は保持しています。" : "部分救出を実行せず保持しました。原音声は変更していません。再起動後も未救出の候補として残ります。");
      if (receipt.recovered) void latest.current.onRecovered(receipt.summary.meetingId).catch(cause => setError(`救出は保存済みです。会議一覧を再読込してください: ${String(cause)}`));
      void latest.current.operation.refresh().catch(cause => setError(String(cause)));
    }
  }, [s, rpcBusy, locked, unknown, confirm.pending, savedActive]);
  const items = [...(operation.partialRecordingListing?.items ?? []), ...(extra?.items ?? [])];
  const next = extra ? extra.next : operation.partialRecordingListing?.next;
  const canRecover = canAct && s?.phase === "ready" && !!preview && !!s.retryAllowed && understood === preview.confirmationId && canRecoverPartialRecording(preview.summary);
  const show = visible || !!items.length || active || unknown || !!error;
  return <>
    <ToastNotice scope="partial-recording" type="result" eventId={toastId} message={toast} onClose={() => setToast("")} />
    {show && <section className="analysis-card recovery-panel" aria-label="中断録音の部分救出">
      <h2 id="partial-recording-heading" tabIndex={-1}>中断録音の部分救出</h2>
      <p>元録音の検証済み確定範囲だけを救出します。録音を再開せず、正常終了や損失5秒以内とは扱いません。</p>
      {!operation.partialRecordingSupported && operation.connected && <p>この環境の部分救出は保留中です。</p>}
      {!operation.connected && <p role="alert">状態を確認できるまで変更を停止しています。{operation.error}</p>}
      <button type="button" className="small-secondary" disabled={rpcBusy} onClick={() => void run(() => latest.current.operation.refresh())}>中断録音と救出結果を再読込</button>
      {operation.partialRecordingSupported && !items.length && <p role="status">現在、未救出の中断録音はありません。</p>}
      <ul>{items.map(item => <li key={item.sessionId}><h3>{item.title}</h3><p>{item.state === "failed" ? "失敗した録音" : "中断した録音"} · 会議ID {item.meetingId}</p>
        <button type="button" className="small-secondary" disabled={!canAct} onClick={() => void choose(item)}>確定範囲と救出方法を確認</button></li>)}</ul>
      {next && <button type="button" className="small-secondary" disabled={pageBusy || !operation.connected} onClick={() => void loadMore()}>次の中断録音を表示</button>}
      {active && s.busy && <><p role="status">{dataPhaseLabel(s.phase)}。停止要求後も、実際の処理終了まで保護を維持します。</p><button type="button" className="stop-button" disabled={rpcBusy || s.phase === "cancelling"} onClick={() => void cancel()}>部分救出の安全な停止を要求</button></>}
      {preview && !s?.busy && !receipt && <div className="analysis-actions"><button type="button" className="small-secondary" disabled={!canAct} onClick={() => { opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; setOpen(true); }}>部分救出の確認内容を開く</button><button type="button" className="small-secondary" disabled={!canAct} onClick={() => void submit("refresh")}>確定範囲と保存結果を再確認</button></div>}
      {unknown && <button type="button" disabled={rpcBusy || locked || !!s?.busy} onClick={() => void retryUnknown()}>同じ部分救出操作の結果を確認</button>}
      <IssueNotice scope="partial-recording" type="operation" label="部分救出" message={error || (active ? s.errorMessage : "")} />
    </section>}
    <PartialRecordingResultsPanel operation={operation} locked={locked || rpcBusy || unknown || confirm.pending || open || !!s?.busy || !!s?.restartRequired} onProtectionChange={setSavedActive} />
    <CommonModal ref={dialog} open={open} blocked={protectedNow || !!s?.busy} onDismiss={cancel}
      restoreReady={!rpcBusy && !s?.busy && operation.connected} restoreTarget={() => opener.current?.isConnected && !opener.current.matches(":disabled") ? opener.current : document.getElementById("partial-recording-heading")}
      className="import-dialog data-dialog" aria-labelledby="partial-recording-title">
      <h2 id="partial-recording-title" tabIndex={-1}>確定済み音声だけを救出する確認</h2>
      {preview && <><p>会議ID {preview.summary.meetingId}<br />録音ID {preview.summary.sessionId}</p><PartialRecordingSummary summary={preview.summary} />
        <label className="import-checkbox"><input type="checkbox" checked={understood === preview.confirmationId} disabled={!canAct || s?.phase !== "ready"} onChange={e => setUnderstood(e.target.checked ? preview.confirmationId : null)} />正常終了・取得終了・末尾損失は不明のまま、検証済み部分だけを救出することを確認しました。</label></>}
      <div className="analysis-actions"><button type="button" disabled={!canRecover} onClick={() => void submit("recover")}>同意して確定済み部分を救出</button>
        <button type="button" className="secondary" disabled={!canAct || s?.phase !== "ready" || !!receipt} onClick={() => void submit("keep")}>救出せず保持</button>
        <button type="button" className="small-secondary" disabled={!canAct || !preview || !!receipt} onClick={() => void submit("refresh")}>最新の範囲を再確認</button>
        <button type="button" className="small-secondary" disabled={protectedNow || !!s?.busy} data-dialog-cancel onClick={() => void cancel()}>救出せず閉じる</button></div>
      {unknown && <button type="button" disabled={rpcBusy || locked || !!s?.busy} onClick={() => void retryUnknown()}>同じ部分救出操作の結果を確認</button>}
      {(error || (active && s.errorMessage)) && <p role="alert">{error || s?.errorMessage}</p>}
    </CommonModal>
  </>;
}
