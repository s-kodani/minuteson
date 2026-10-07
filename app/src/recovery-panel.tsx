import { invoke, isTauri } from "@tauri-apps/api/core";
import { InfoPopover } from "./info-popover";
import { IssueNotice, ToastNotice } from "./toast";
import { CommonModal } from "./common-modal";
import { useActionConfirmation } from "./confirmation-service";
import { withConfirmedEdits } from "./confirmed-action";
import { waitForDataStop } from "./data-stop";
import { useEffect, useRef, useState } from "react";
import type { DataOperation } from "./data-operation";
import { dataRequest, type DataRequest, type DataSnapshot } from "./data-operation-state";
import { assertRecoveryListing, recoveryOutcomeLabel, recoveryOutcomeStatus, recoverySourceLabel, type RecoveryItem, type RecoveryListing } from "./recovery-state";

class UnconfirmedRecovery extends Error {}

export function RecoveryPanel({ operation, visible, locked, withSavedEdits, onProtectionChange }: {
  operation: DataOperation; visible: boolean; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  onProtectionChange: (active: boolean) => void;
}) {
  const latest = useRef(operation); latest.current = operation;
  const confirm = useActionConfirmation(false);
  const conditions = useRef({ locked, unknown: false });
  const s = operation.snapshot;
  const [listing, setListing] = useState<RecoveryListing | null>(null);
  const [error, setError] = useState("");
  const [rpcBusy, setRpcBusy] = useState(false);
  const rpcRunning = useRef(false);
  const [unknown, setUnknown] = useState(false);
  const [uncertaintyMessage, setUncertaintyMessage] = useState("");
  conditions.current = { locked, unknown };
  const [open, setOpen] = useState(false);
  const pending = useRef<DataRequest | null>(null);
  const pendingChoice = useRef<{ item: RecoveryItem; revision: number } | null>(null);
  const review = useRef<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const panelHeading = useRef<HTMLHeadingElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const listEpoch = useRef(0);
  const readList = async () => {
    const epoch = ++listEpoch.current;
    try { const value = await invoke<RecoveryListing>("get_recovery_items"); assertRecoveryListing(value); if (epoch === listEpoch.current) setListing(value); }
    catch (cause) { if (epoch === listEpoch.current) setListing(null); throw cause; }
  };
  const run = async (action: () => Promise<void>, propagate = false) => {
    if (rpcRunning.current) { if (propagate) throw Error("別の回収操作が実行中です。処理は開始していません。"); return; }
    rpcRunning.current = true; setRpcBusy(true); setError("");
    try { await action(); } catch (cause) {
      // A state event may confirm admission before the lost RPC reply reaches
      // this catch. Do not resurrect uncertainty after that verified event.
      if (cause instanceof UnconfirmedRecovery) setUncertaintyMessage(cause.message);
      else setError(String(cause));
      if (propagate) throw cause;
    }
    finally { rpcRunning.current = false; setRpcBusy(false); }
  };
  useEffect(() => { onProtectionChange(rpcBusy || unknown || confirm.pending); }, [rpcBusy, unknown, confirm.pending, onProtectionChange]);
  useEffect(() => {
    if (visible && isTauri()) void readList().catch(cause => setError(String(cause)));
    return () => { listEpoch.current++; };
  }, [visible, s?.kind === "recovery" && s.phase === "completed" ? s.revision : -1]);
  useEffect(() => {
    if (!s || s.kind !== "recovery") { setOpen(false); return; }
    if (s.busy || s.phase === "completed") setOpen(false);
    if (unknown) {
      const choice = pendingChoice.current;
      if ((choice && s.revision > choice.revision && (s.busy || s.phase === "failed" || (s.recoveryPreview?.item.operationId === choice.item.operationId && s.recoveryPreview.item.source === choice.item.source)))
        || (pending.current?.token === s.token && (s.busy || ["completed", "failed", "cancelled"].includes(s.phase)))) { pending.current = null; pendingChoice.current = null; setUnknown(false); }
    }
    if (s.phase === "ready" && s.recoveryPreview && visible && !rpcBusy && !locked && !unknown && !confirm.pending && review.current !== s.recoveryPreview.confirmationId) { review.current = s.recoveryPreview.confirmationId; setOpen(true); }
  }, [s, visible, rpcBusy, locked, unknown, confirm.pending]);
  const send = async (request: DataRequest) => {
    pending.current = request;
    try { latest.current.accept(await invoke<DataSnapshot>("start_data_operation", { request })); pending.current = null; setUnknown(false); setOpen(false); }
    catch (cause) { setUnknown(true); throw new UnconfirmedRecovery(`受付結果が未確定です。同じ操作の結果を確認してください: ${String(cause)}`); }
  };
  const choose = async (item: RecoveryItem) => {
    if (!canAct || item.retained) return;
    const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const expected = latest.current.getSnapshot();
    const sameTarget = () => latest.current.connected && !!latest.current.availability?.available && latest.current.getSnapshot()?.revision === expected?.revision && !latest.current.getSnapshot()?.busy;
    const inspect = async (propagate = false) => {
      const action = async () => run(async () => {
        if (propagate && !sameTarget()) throw Error("確認対象が変更されました。元の画面で再確認してください。検査は開始していません。");
        opener.current = active; setOpen(false);
        pendingChoice.current = { item, revision: latest.current.snapshot?.revision ?? -1 };
        try { latest.current.accept(await invoke<DataSnapshot>("review_recovery_item", { source: item.source, operationId: item.operationId })); pendingChoice.current = null; setUnknown(false); }
        catch (cause) { setUnknown(true); throw new UnconfirmedRecovery(`検査の受付を確認できません。まだ回収・保持確認は行っていません: ${String(cause)}`); }
      }, propagate);
      if (propagate) await withConfirmedEdits(withSavedEdits, action); else await withSavedEdits(action);
    };
    if (expected?.retryAllowed) await confirm({ opener: active, title: "回収対象の確認内容変更を確認", target: `${recoverySourceLabel(item.source)} · ${item.operationId}`, confirmLabel: "確認内容を破棄して対象を検査する", message: "現在の確認内容を破棄して、要確認対象を検査します。既存会議と元バックアップは変更しません。検査だけでは回収・保持確認を行いません。", validate: () => !conditions.current.locked && !conditions.current.unknown && !rpcRunning.current && sameTarget(), action: () => inspect(true) });
    else await inspect();
  };
  const submit = async (action: "recover" | "keep" | "refresh") => {
    if (!canAct) return;
    await withSavedEdits(async () => run(async () => { const now = latest.current.getSnapshot(); if (!now) throw Error("操作を再確認してください"); await send(dataRequest(now, action)); }));
  };
  const retryUnknown = async () => {
    if (rpcRunning.current || locked || s?.busy) return;
    await withSavedEdits(async () => run(async () => {
      if (pending.current) await send(pending.current);
      else { await latest.current.refresh(); const now = latest.current.getSnapshot(), choice = pendingChoice.current;
        if (choice && now?.kind === "recovery" && now.revision > choice.revision
          && (now.busy || now.phase === "failed" || (now.recoveryPreview?.item.operationId === choice.item.operationId && now.recoveryPreview.item.source === choice.item.source))) { pendingChoice.current = null; setUnknown(false); }
        else throw new UnconfirmedRecovery("検査の受付はまだ確認できません。別の操作は開始しないでください。");
      }
    }));
  };
  const cancel = async () => {
    if (rpcRunning.current || !s?.token || s.kind !== "recovery") return;
    const token = s.token;
    const stop = async (propagate = false) => run(async () => { latest.current.accept(await invoke<DataSnapshot>("cancel_data_operation", { token })); if (propagate) await waitForDataStop(token, "recovery", async () => { await latest.current.refresh(); return latest.current.getSnapshot(); }); setOpen(false); pending.current = null; pendingChoice.current = null; setUnknown(false); }, propagate);
    if (s.busy) await confirm({ title: "回収操作の安全な停止を確認", target: s.recoveryPreview ? `${recoverySourceLabel(s.recoveryPreview.item.source)} · ${s.recoveryPreview.item.operationId}` : "回収操作（対象を検査中）", confirmLabel: "安全な停止を要求して待つ", message: "回収操作の安全な停止を待ちます。完了済みの回収や保持確認は取り消しません。未登録・変更済みのファイルは保持し、停止完了まで書込保護を維持します。", validate: () => !rpcRunning.current && latest.current.getSnapshot()?.token === token && latest.current.getSnapshot()?.kind === "recovery" && !!latest.current.getSnapshot()?.busy, action: () => stop(true) });
    else await stop();
  };
  const canAct = isTauri() && !!listing?.supported && operation.connected && !!operation.availability?.available && !locked && !rpcBusy && !confirm.pending && !s?.busy && !unknown;
  const preview = s?.kind === "recovery" ? s.recoveryPreview : null;
  const receipt = s?.kind === "recovery" ? s.recoveryReceipt : null;
  const ready = canAct && s?.phase === "ready" && s.retryAllowed && !!preview;
  const operationError = error || (unknown ? uncertaintyMessage : "") || (s?.kind === "recovery" ? s.errorMessage || (receipt?.outcome === "attention" ? `${recoveryOutcomeLabel(receipt)} ${receipt.reason ?? ""}` : "") : "");
  return <>
    <ToastNotice scope="recovery" type="completed" eventId={s?.token ?? undefined} message={s?.kind === "recovery" && s.phase === "completed" && receipt && receipt.outcome !== "attention" ? recoveryOutcomeLabel(receipt) : ""} />
    {!open && <IssueNotice scope="recovery" type="operation" label="要確認データの確認・回収" message={operationError} />}
    {visible && <section className="analysis-card recovery-panel" aria-label="中断したデータ操作の確認">
      <div className="heading-with-info"><h2 ref={panelHeading} tabIndex={-1}>中断したデータ操作の確認</h2><InfoPopover label="要確認データ" details={<p>未公開の復元追加物と構築用コピーだけが対象です。既存会議・公開済みデータ・元バックアップは変更しません。所有不明のファイルは削除しません。「保持して確認を完了」は回収ではなく、今後の自動回収からも保護する確認記録です。</p>}>未公開の追加復元物と移動途中のコピーを確認します。既存会議は変更しません。保持の確認と、対象を削除する回収は別の操作です。</InfoPopover></div>
      <button type="button" className="small-secondary" disabled={rpcBusy} onClick={() => void run(async () => { await latest.current.refresh(); await readList(); })}>要確認一覧を再読込</button>
      {!listing && <p role="status">一覧を確認できるまで回収・保持確認はできません。</p>}
      {listing && !listing.supported && <p role="status">このOSの回収操作は保留中です。</p>}
      {listing?.supported && <><p>保存場所: <span className="data-path">{listing.root}</span></p><p role="status">未確認 {listing.items.filter(i => !i.retained).length} 件 · 保持して確認済み {listing.items.filter(i => i.retained).length} 件</p><ul className="recovery-list">{listing.items.map(item => <li key={`${item.source}:${item.operationId}`}><h3>{recoverySourceLabel(item.source)}</h3><p>{item.reason}</p>{item.paths.map(path => <p className="data-path" key={path}>{path}</p>)}<p>登録対象 {item.registeredPaths} 件</p>{item.retained ? <p role="status">保持して確認済み。ファイルは削除していません。</p> : <button type="button" className="small-secondary" disabled={!canAct} onClick={() => void choose(item)}>対象と解決方法を確認</button>}</li>)}</ul></>}
      {s?.kind === "recovery" && s.busy && <p role="status">要確認対象を処理中です。停止後も未登録・変更済みのファイルを保持します。</p>}
      {unknown && !open && <button type="button" disabled={rpcBusy || locked || !!s?.busy} onClick={() => void retryUnknown()}>同じ確認操作の結果を確認</button>}
      {s?.kind === "recovery" && s.busy && <button type="button" className="small-secondary" disabled={rpcBusy || s.phase === "cancelling"} onClick={() => void cancel()}>安全な停止を要求</button>}
      {preview && s?.retryAllowed && !s.busy && !unknown && <div className="analysis-actions"><button type="button" className="small-secondary" disabled={!canAct} onClick={() => { opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; setOpen(true); }}>回収の確認内容を開く</button><button type="button" className="small-secondary" disabled={!canAct} onClick={() => void submit("refresh")}>最新の対象を再確認</button><button type="button" className="small-secondary" disabled={rpcBusy} onClick={() => void cancel()}>確認だけを取り消す</button></div>}
      {receipt && <p role="status">{recoveryOutcomeStatus(receipt)}</p>}
    </section>}
    <CommonModal ref={dialog} open={open} blocked={unknown || rpcBusy || !!s?.busy} onDismiss={cancel} restoreReady={visible && !rpcBusy && !locked && !s?.busy && operation.connected} restoreTarget={() => s?.kind === "recovery" && s.recoveryReceipt ? panelHeading.current : opener.current?.isConnected && !opener.current.matches(":disabled") ? opener.current : panelHeading.current} className="import-dialog data-dialog" aria-labelledby="recovery-confirm-title">
      <h2 id="recovery-confirm-title" ref={heading} tabIndex={-1}>未確定データの解決方法を確認</h2>
      {preview && <><h3>{recoverySourceLabel(preview.item.source)}</h3><p>{preview.item.reason}</p><p>保存場所: <span className="data-path">{preview.root}</span></p>{preview.item.paths.map(path => <p className="data-path" key={path}>{path}</p>)}<p>登録対象 {preview.item.registeredPaths} 件 · 現在確認できたパス {preview.observedPaths} 件</p>{!preview.inspectionComplete && <p role="alert">リンクや読取不可の場所があり、内容をすべて検査できません。所有不明の対象は保持します。</p>}</>}
      <p>再検査は所有を確認できた未公開物だけを回収し、結果を再表示します。保持を選ぶとファイルは変更せず、確認記録を残して自動回収から保護します。会議の完全削除や外部バックアップの削除ではありません。</p>
      <div className="analysis-actions"><button type="button" disabled={!ready} onClick={() => void submit("recover")}>再検査して回収</button><button type="button" className="secondary" disabled={!ready} onClick={() => void submit("keep")}>保持して確認を完了</button><button type="button" className="small-secondary" disabled={!canAct || !s?.retryAllowed} onClick={() => void submit("refresh")}>最新の対象を再確認</button><button type="button" className="small-secondary" disabled={rpcBusy || unknown || !!s?.busy} data-dialog-cancel onClick={() => void cancel()}>取り消して閉じる</button></div>
      {unknown && <div className="analysis-actions"><button type="button" disabled={rpcBusy || locked || !!s?.busy} onClick={() => void retryUnknown()}>同じ確認操作の結果を確認</button><button type="button" className="small-secondary" disabled={rpcBusy || !!s?.busy} onClick={() => void cancel()}>同じ確認操作を取り消す</button></div>}
      {operationError && <p role="alert">{operationError}</p>}
    </CommonModal>
  </>;
}
