import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { DataOperation } from "./data-operation";
import { dataPhaseLabel, dataRequest, type DataKind, type DataRequest, type DataSnapshot, type RestoreReceipt } from "./data-operation-state";
import { bytesLabel } from "./import-state";
import { assertStorageView, type StorageView } from "./storage-placement-state";

export function DataSavePanel({ operation, visible, locked, withSavedEdits, onProtectionChange, onRestored, onShowMeetings }: {
  operation: DataOperation; visible: boolean; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  onProtectionChange: (active: boolean) => void;
  onRestored: (receipt: RestoreReceipt) => Promise<void>;
  onShowMeetings: (ids: string[]) => Promise<void>;
}) {
  const { snapshot: s, connected, availability } = operation;
  const latest = useRef(operation); latest.current = operation;
  const [rpcBusy, setRpcBusy] = useState(false);
  const rpcRunning = useRef(false);
  const [error, setError] = useState("");
  const [unknown, setUnknown] = useState(false);
  const [open, setOpen] = useState(false);
  const [toast, setToast] = useState("");
  const [openError, setOpenError] = useState("");
  const pending = useRef<DataRequest | null>(null);
  const pendingChoice = useRef<{ kind: DataKind; revision: number } | null>(null);
  const autoBackup = useRef<string | null>(null);
  const completed = useRef<string | null>(null);
  const review = useRef<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const restoreFocus = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const [storage, setStorage] = useState<StorageView | null>(null);
  const [storageError, setStorageError] = useState("");
  useEffect(() => {
    if (!visible || !isTauri()) return;
    let disposed = false;
    void invoke<StorageView>("get_storage_bootstrap").then(value => {
      assertStorageView(value);
      if (!disposed) { setStorage(value); setStorageError(""); }
    }).catch(cause => { if (!disposed) { setStorage(null); setStorageError(String(cause)); } });
    return () => { disposed = true; };
  }, [visible, s?.revision]);
  useEffect(() => { onProtectionChange(rpcBusy || unknown); }, [rpcBusy, unknown, onProtectionChange]);
  useEffect(() => {
    if (open && dialog.current && !dialog.current.open) { restoreFocus.current = false; dialog.current.showModal(); heading.current?.focus(); }
    else if (!open && dialog.current?.open) { dialog.current.close(); restoreFocus.current = true; }
    if (!open && restoreFocus.current && !rpcBusy && !locked && !s?.busy && connected && availability?.available && opener.current?.isConnected && !opener.current.matches(":disabled")) {
      opener.current.focus(); restoreFocus.current = false;
    }
  }, [open, rpcBusy, locked, s?.busy, connected, availability?.available]);
  const run = async (action: () => Promise<void>) => {
    if (rpcRunning.current) return;
    rpcRunning.current = true; setRpcBusy(true); setError("");
    try { await action(); }
    catch (cause) { setError(String(cause)); }
    finally { rpcRunning.current = false; setRpcBusy(false); }
  };
  const read = async () => { await latest.current.refresh(); };
  const send = async (request: DataRequest) => {
    pending.current = request;
    try {
      latest.current.accept(await invoke<DataSnapshot>("start_data_operation", { request }));
      pending.current = null; setUnknown(false); setOpen(false);
    } catch (cause) {
      setUnknown(true);
      throw Error(`受付結果を確定できません。別の操作を始めず、同じ操作の結果を確認してください: ${String(cause)}`);
    }
  };
  const submit = async (action: DataRequest["action"]) => {
    if (rpcRunning.current || locked || !latest.current.connected || latest.current.snapshot?.busy) return;
    await withSavedEdits(async () => run(async () => {
      const current = latest.current.getSnapshot();
      if (!current) throw Error("操作の状態を再確認してください。");
      const request = dataRequest(current, action);
      await send(request);
    }));
  };
  const choose = async (kind: DataKind, pendingMove = false) => {
    if (rpcRunning.current || locked || unknown || s?.busy || !connected || !availability?.available) return;
    if (s?.attentionRequired && !window.confirm(pendingMove ? "未完了移動の現在の内容を再確認しますか？ 確認だけでは切替・削除しません。" : "前の保存結果は未確定です。保存先の実ファイルを確認してから、別の操作を開始しますか？ 前の結果と途中コピーは変更しません。")) return;
    if (s?.retryAllowed && !window.confirm("現在の確認内容を破棄して選び直しますか？ 既存会議と元バックアップは変更しません。")) return;
    const selectionOpener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    await withSavedEdits(async () => run(async () => {
      opener.current = selectionOpener;
      setOpen(false); setToast(""); setOpenError("");
      pending.current = null; pendingChoice.current = { kind, revision: latest.current.snapshot?.revision ?? -1 };
      try {
        latest.current.accept(pendingMove ? await invoke<DataSnapshot>("review_storage_move") : await invoke<DataSnapshot>("select_data_operation", { kind }));
        pendingChoice.current = null; setUnknown(false);
      } catch (cause) {
        setUnknown(true); throw Error(`選択の受付状態を確認してください。新しい復元はまだ確定していません: ${String(cause)}`);
      }
    }));
  };
  const retryUnknown = async () => {
    if (rpcRunning.current || locked) return;
    await withSavedEdits(async () => run(async () => {
      if (pending.current) await send(pending.current);
      else {
        await read();
        const now = latest.current.getSnapshot(), choice = pendingChoice.current;
        if (choice && now && now.revision > choice.revision) { pendingChoice.current = null; setUnknown(false); }
        else throw Error("選択はまだ受付を確認できません。状態の再確認を続けてください。");
      }
    }));
  };
  const cancel = async () => {
    const current = latest.current.getSnapshot();
    if (rpcRunning.current || !current?.token) return;
    if (current.busy && !window.confirm(current.kind === "relocation" ? "保存先変更を取り消しますか？ 旧保存先と途中コピーを保持します。切替済みの場合は終了して再起動が必要です。" : "進行中の処理を取り消しますか？ 公開済みの結果と既存会議は保持し、未公開の追加物だけを整理します。")) return;
    await run(async () => {
      latest.current.accept(await invoke<DataSnapshot>("cancel_data_operation", { token: current.token }));
      setOpen(false); pending.current = null; pendingChoice.current = null; setUnknown(false);
    });
  };
  const dismiss = async () => {
    if (rpcRunning.current || unknown || latest.current.snapshot?.busy) { setError("処理と結果確認が終わってから閉じてください。"); return; }
    await cancel();
  };
  useEffect(() => {
    if (!s) return;
    if (s.kind === "recovery" || s.kind === "deletion" || s.kind === "model_deletion" || s.kind === "audio_deletion") { setOpen(false); setToast(""); return; }
    if (s.busy) setOpen(false);
    if (unknown) {
      const choice = pendingChoice.current;
      if ((choice && s.revision > choice.revision) || (pending.current?.token === s.token && (s.busy || ["completed", "failed", "cancelled", "attention"].includes(s.phase)))) {
        if (choice) pendingChoice.current = null;
        if (!s.busy) pending.current = null;
        setUnknown(false); if (s.phase === "completed") setError("");
      }
    }
    if (s.phase === "completed" && s.token !== completed.current) {
      completed.current = s.token; setOpen(false); setError("");
      setToast(s.relocationReceipt ? s.relocationReceipt.retained ? "途中コピーを保持して移動を中止しました。旧保存先を引き続き使用します。ファイルは削除していません。" : "保存先を切り替えました。終了して再起動してください。旧保存先は削除していません。" : s.restoreReceipt ? `会議 ${s.restoreReceipt.meetingIds.length} 件を追加復元しました。` : "バックアップを作成しました。");
      if (s.restoreReceipt) void onRestored(s.restoreReceipt).catch(cause => setOpenError(`復元は完了しています。会議一覧を再読込してください: ${String(cause)}`));
    }
    const confirmation = s.backupPreview?.confirmationId ?? s.restorePreview?.confirmationId ?? s.relocationPreview?.confirmationId;
    if (s.phase === "ready" && confirmation && !rpcBusy && !locked && !unknown) {
      if (s.kind === "backup" && !s.backupPreview?.replacesExisting) {
        if (autoBackup.current !== confirmation && connected && availability?.available) {
          autoBackup.current = confirmation; void submit("publish");
        }
      } else if (review.current !== confirmation && visible) {
        review.current = confirmation; setOpen(true);
      }
    }
  }, [s, rpcBusy, locked, unknown, connected, availability?.available, visible]);
  const canAct = isTauri() && connected && !locked && !rpcBusy && !s?.busy && !s?.restartRequired && !storage?.restartRequired && !unknown && !!availability?.available;
  const requestReady = s?.kind !== "recovery" && s?.kind !== "deletion" && s?.kind !== "model_deletion" && s?.kind !== "audio_deletion" && !!s?.retryAllowed && !s.attentionRequired;
  const restore = s?.restorePreview;
  const moving = s?.relocationPreview;
  const canPublish = canAct && requestReady && (s?.kind === "backup" || (s?.phase === "ready" && ((!!restore && restore.availableBytes >= restore.requiredBytes) || (!!moving?.resumeAllowed && moving.availableBytes >= moving.requiredBytes))));
  const show = async () => run(async () => { setOpenError(""); if (s?.restoreReceipt) await onShowMeetings(s.restoreReceipt.meetingIds); });
  return <>
    {visible && <section className="analysis-card data-save" aria-label="データと保存">
      <div className="card-head"><h2>データと保存</h2><details className="data-info"><summary aria-label="バックアップと追加復元の説明">info</summary><p>会議・管理音声・文字起こし・議事録・下書き・用語を保存します。AIモデル本体・一時PCM・ログ・端末固有の設定は含みません。追加復元は現在の会議を置き換えず、同じ会議も新しい別会議として追加します。独自暗号化やクラウド同期ではありません。内容を含むファイルは安全な場所へ保管してください。</p></details></div>
      <div className="analysis-actions"><button type="button" disabled={!canAct} onClick={() => void choose("backup")}>バックアップを作成</button><button type="button" className="secondary" disabled={!canAct} onClick={() => void choose("restore")}>バックアップから追加復元</button></div>
      {storage && <><p>会議保存先: <span className="data-path">{storage.meetingRoot}</span></p><p>AIモデル保存先（移動しません）: <span className="data-path">{storage.modelsRoot}</span></p><button type="button" className="secondary" disabled={!canAct || !!storage.pendingOperationId} onClick={() => void choose("relocation")}>会議保存先を変更</button>{storage.pendingOperationId && <><p role="alert">未完了の保存先変更があります。旧保存先と途中コピーを保持しています。新たな保存先変更は開始しません。途中コピー: <span className="data-path">{storage.pendingTarget}</span></p><button type="button" disabled={!canAct} onClick={() => void choose("relocation", true)}>未完了移動を確認</button></>}{storage.keptMoves.length > 0 && <details><summary>保持した途中コピー（{storage.keptMoves.length} 件）</summary><p>利用中の会議ではありません。ファイルを削除・採用しません。別の保存先変更を開始しても残ります。</p><ul>{storage.keptMoves.map(m => <li key={m.operationId}><span className="data-path">{m.destination}</span> — {m.decisionPending ? "保持判断を保存済み。未完了移動を確認して中止を完了してください。" : "保持して中止済み"}</li>)}</ul></details>}</>}
      {storageError && <p role="status">保存先の確認・変更は利用できません: {storageError}</p>}
      {(s?.restartRequired || storage?.restartRequired) && <p role="alert">保存先切替の書込停止中です。新しい録音・編集・AI処理は開始できません。アプリを終了して再起動してください。現在開いている保存先: <span className="data-path">{storage?.activeMeetingRoot ?? moving?.source}</span>。旧保存先は削除しません。</p>}
      {(s?.restartRequired || storage?.restartRequired) && <button type="button" disabled={rpcBusy || !!s?.busy} onClick={() => void run(async () => { if (window.confirm("アプリを安全に終了しますか？ 終了後、手動で再起動してください。旧保存先は削除しません。")) await invoke("finish_main_close", { quit: true }); })}>終了する（再起動は手動）</button>}
      {!connected && <p role="status">データ操作の状態を確認できるまで変更操作を停止しています。</p>}
      {availability?.reason && !s?.busy && <p role="status">{availability.reason}。停止・完了後にバックアップ／復元できます。</p>}
      {s?.token && s.kind !== "recovery" && s.kind !== "deletion" && s.kind !== "model_deletion" && s.kind !== "audio_deletion" && <p role="status">{dataPhaseLabel(s.phase)}{s.busy && "。未完成ファイルは利用できません。"}</p>}
      <div className="analysis-actions">
        <button type="button" className="small-secondary" disabled={rpcBusy} onClick={() => void run(read)}>状態を再確認</button>
        {unknown && <><button type="button" disabled={rpcBusy || locked || !!s?.busy} onClick={() => void retryUnknown()}>同じ操作の結果を確認</button><button type="button" className="small-secondary" disabled={rpcBusy || !s?.token} onClick={() => void cancel()}>同じ操作を取り消して確認</button></>}
        {s?.busy && s.kind !== "deletion" && s.kind !== "model_deletion" && s.kind !== "audio_deletion" && <button type="button" className="stop-button" disabled={rpcBusy || ["cancelling", "discarding"].includes(s.phase)} onClick={() => void cancel()}>処理を取り消す</button>}
        {requestReady && !s?.busy && !unknown && <><button type="button" className="small-secondary" disabled={!canAct} onClick={() => { opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; setOpen(true); }}>確認内容を開く</button>{s?.kind === "restore" && <button type="button" className="small-secondary" disabled={!canAct} onClick={() => void submit("refresh")}>同じバックアップを再検証</button>}{s?.kind === "relocation" && <button type="button" className="small-secondary" disabled={!canAct} onClick={() => void submit("refresh")}>同じ保存先を再確認</button>}<button type="button" className="small-secondary" disabled={rpcBusy} onClick={() => void cancel()}>確認内容を破棄</button></>}
      </div>
      {(error || operation.error || (s?.kind !== "deletion" && s?.errorMessage)) && <p className="action-error" role="alert">{error || operation.error || (s?.kind !== "deletion" && s?.errorMessage)}</p>}
      {s?.attentionRequired && s.kind !== "deletion" && s.kind !== "model_deletion" && s.kind !== "audio_deletion" && <p role="alert">{s.kind === "relocation" ? "保存先変更の確認が必要です。旧保存先と途中コピーを保持し、自動削除・再上書きは行いません。切替結果が未確定の場合は終了して再起動してください。" : "保存先に公開済みの可能性があります。実ファイルを確認してください。同じ操作での再上書きは行いません。"}</p>}
      {toast && <p className="data-toast" role="status">{toast}<button type="button" className="small-secondary" onClick={() => setToast("")}>通知を閉じる</button></p>}
      {s?.backupReceipt && <><p>保存先: <span className="data-path">{s.backupPreview?.destination}</span><br />会議 {s.backupReceipt.meetingCount} 件 · {bytesLabel(s.backupReceipt.size)}</p>{s.backupReceipt.deletedAudioMeetingCount > 0 && <p role="alert">音声削除済みの会議 {s.backupReceipt.deletedAudioMeetingCount} 件・音声資産 {s.backupReceipt.deletedAudioAssetCount} 件の欠落理由と本文・根拠の履歴を保存しました。音声本体は含みません。追加復元後も、削除済み音声の再生・再解析はできません。</p>}</>}
      {s?.restoreReceipt && <>{s.restoreReceipt.deletedAudioMeetingCount > 0 && <p role="alert">音声削除済みの会議 {s.restoreReceipt.deletedAudioMeetingCount} 件・音声資産 {s.restoreReceipt.deletedAudioAssetCount} 件は、本文・根拠・下書きと欠落理由の履歴のみ復元しました。削除済み音声の再生・再解析はできません。</p>}<button type="button" disabled={rpcBusy || locked} onClick={() => void show()}>復元した会議一覧を開く</button><p>共通用語：追加 {s.restoreReceipt.vocabularyAdded ?? "記録不明"} 件、完全一致の重複 {s.restoreReceipt.vocabularyDeduplicated ?? "記録不明"} 件、既存を保持した競合 {s.restoreReceipt.vocabularyConflicts?.length ?? "記録不明"} 件。</p><p>共通用語の表示を更新するには、編集を保存して「用語を再読込」を選んでください。</p>{s.restoreReceipt.vocabularyConflicts && s.restoreReceipt.vocabularyConflicts.length > 0 && <details><summary>競合した用語を確認</summary><ul>{s.restoreReceipt.vocabularyConflicts.map(term => <li key={term}>{term}</li>)}</ul></details>}</>}
      {openError && <p role="alert">{openError}</p>}
    </section>}
    <dialog ref={dialog} className="import-dialog data-dialog" aria-labelledby="data-confirm-title" onCancel={event => { event.preventDefault(); void dismiss(); }}>
      <h2 id="data-confirm-title" ref={heading} tabIndex={-1}>{s?.kind === "relocation" ? moving?.pending ? "未完了移動の確認" : "会議保存先の変更確認" : s?.kind === "backup" ? "バックアップの保存先確認" : "バックアップから追加復元"}</h2>
      {moving && <><p>{moving.pending ? "続行する場合は元データ・所有台帳・途中コピーを再照合してから切り替えます。「保持して移動を中止」はファイルを消さず、旧保存先を引き続き使います。" : "会議DBと管理音声をコピーして全対象を照合し、保存先を切り替えます。"}切替後は終了して再起動が必要です。旧保存先は削除せず、モデルと起動設定は移動しません。</p><dl className="import-info"><dt>現在の会議保存先</dt><dd className="data-path">{moving.source}</dd><dt>{moving.pending ? "途中コピーの保存先" : "新規作成する保存先"}</dt><dd className="data-path">{moving.destination}</dd><dt>{moving.pending ? "旧保存先の現在の対象" : "対象"}</dt><dd>会議 {moving.meetingCount} 件・ファイル {moving.fileCount ?? "確認不可"} 件</dd>{moving.copiedPaths !== null && <><dt>照合済みパス</dt><dd>{moving.copiedPaths} / {moving.expectedPaths}</dd></>}{moving.fileCount !== null && <><dt>必要容量（余裕を含む）</dt><dd>{bytesLabel(moving.requiredBytes)}</dd><dt>移動先の空き容量</dt><dd>{bytesLabel(moving.availableBytes)}</dd></>}</dl>{moving.pending ? <p>不明な既存物を採用・削除しません。確認後に対象が変わった場合は再確認してください。</p> : <p>選んだ親フォルダーの下に非公開の新規フォルダーを作成します。既存物は上書きしません。確認後に編集した場合は再確認してください。</p>}{moving.resumeReason && <p role="alert">続行できません: {moving.resumeReason}。コピーを保持して移動を中止できます。</p>}{moving.availableBytes < moving.requiredBytes && <p role="alert">移動先の空き容量が不足しています。容量を確保して再確認してください。</p>}</>}
      {s?.backupPreview && <><p>{s.backupPreview.replacesExisting ? "次の既存ファイルを上書きします。元のファイルは置き換えられます。" : "次の保存先にバックアップを作成します。"}</p><p className="data-path">{s.backupPreview.destination}</p></>}
      {restore && <>{restore.deletedAudioMeetingCount > 0 && <p role="alert">音声削除済みの会議 {restore.deletedAudioMeetingCount} 件・音声資産 {restore.deletedAudioAssetCount} 件を含みます。本文・根拠・下書きと欠落理由の履歴を追加しますが、削除済み音声の再生・再解析はできません。</p>}<p>会議 {restore.meetingTitles.length} 件を追加します。現在の会議は置き換えません。同じ会議も新しい別会議として復元されます。</p><dl className="import-info"><dt>必要容量（余裕を含む）</dt><dd>{bytesLabel(restore.requiredBytes)}</dd><dt>保存先の空き容量</dt><dd>{bytesLabel(restore.availableBytes)}</dd></dl><details><summary>追加する会議</summary><ul>{restore.meetingTitles.map((title, i) => <li key={i}>{title}</li>)}</ul></details><p>共通用語：追加予定 {restore.vocabularyAdded} 件、完全一致の重複 {restore.vocabularyDeduplicated} 件、既存を保持する競合 {restore.vocabularyConflicts.length} 件。AIモデル本体は追加しません。</p>{restore.vocabularyConflicts.length > 0 && <details><summary>競合予定の用語</summary><ul>{restore.vocabularyConflicts.map(term => <li key={term}>{term}</li>)}</ul></details>}{restore.availableBytes < restore.requiredBytes && <p role="alert">保存先の空き容量が不足しています。容量を確保して再検証してください。</p>}</>}
      {(error || operation.error || (s?.kind !== "deletion" && s?.errorMessage)) && <p className="action-error" role="alert">{error || operation.error || (s?.kind !== "deletion" && s?.errorMessage)}</p>}
      <p>編集内容の保存を待ってから実行します。公開後の結果は取消で削除しません。</p>
      <div className="analysis-actions"><button type="button" disabled={!canPublish} onClick={() => void submit("publish")}>{s?.kind === "relocation" ? moving?.pending ? "照合して移動を続行" : "コピーして保存先を切替" : s?.kind === "backup" ? s.backupPreview?.replacesExisting ? "上書きして作成" : "作成する" : "追加して復元"}</button>{moving?.pending && <button type="button" className="secondary" disabled={!canAct || !requestReady || s?.phase !== "ready"} onClick={() => void submit("keep")}>保持して移動を中止</button>}{unknown && <><button type="button" disabled={rpcBusy || locked || !!s?.busy} onClick={() => void retryUnknown()}>同じ操作の結果を確認</button><button type="button" className="small-secondary" disabled={rpcBusy || !s?.token} onClick={() => void cancel()}>同じ操作を取り消して確認</button></>}{s?.kind === "restore" && <button type="button" className="small-secondary" disabled={!canAct || !requestReady} onClick={() => void submit("refresh")}>同じバックアップを再検証</button>}{s?.kind === "relocation" && <button type="button" className="small-secondary" disabled={!canAct || !requestReady} onClick={() => void submit("refresh")}>同じ保存先を再確認</button>}<button type="button" className="small-secondary" disabled={rpcBusy || !!s?.busy || unknown} onClick={() => void dismiss()}>取り消して閉じる</button></div>
    </dialog>
  </>;
}
