import { useEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";
import { checkedStorageWriteState, type StorageWriteState } from "./data-operation-state";
import { flushPendingEdits, pendingEditsSnapshot } from "./draft-save";
import type { SalvageSourceCleanup } from "./generated/contracts";
import { validateSalvageSourceCleanup } from "./generated/validators.js";
import { invoke, isTauri } from "./ipc";
import { displayNativePathsInText, displayNativePath, nativeAbsolutePath } from "./native-path";
import { ErrorNotice } from "./toast";
import "./salvage.css";

type Item = {
  kind: string;
  sourceId: string | null;
  label: string;
  status: "recoverable" | "partial" | "unavailable" | "uninspected" | "defaulted" | "absent" | "deleted";
  reason: string | null;
};
type Preview = {
  operationId: string;
  sourceRoot: string;
  requiredBytes: number;
  meetingTotal: number | null;
  items: Item[];
  recoverableCount: number;
};
type Result = {
  operationId: string;
  sourceRoot: string;
  sourceModels: string | null;
  destinationRoot: string;
  destinationModels: string;
  meetingCount: number;
  voicePersonCount: number;
  modelCount: number;
  items: Item[];
};
type Status = {
  phase: "idle" | "opening" | "inspecting" | "building";
  error: string | null;
  progress?: { stage: string; completed: number; total: number | null; unit: string } | null;
  selection: {
    operationId: string | null;
    sourceRoot: string | null;
    destinationRoot: string | null;
    availableBytes: number | null;
    sourceSelectable: boolean;
    preview: Preview | null;
    result: Result | null;
  } | null;
};
type EditExport =
  | { status: "failed"; message: string }
  | { status: "cancelled" }
  | { status: "saved"; contents: string };
const itemStates: Record<Item["status"], string> = {
  recoverable: "復元可能",
  partial: "一部復元可能",
  unavailable: "復元不可",
  uninspected: "未検査",
  defaulted: "既定値を使用",
  absent: "データなし",
  deleted: "削除済み",
};
const itemKinds: Record<string, string> = {
  meeting: "会議", transcript: "文字起こし", draft: "文字起こしの下書き", minutes: "議事録", audio: "音声",
  voice_person: "登録した声", voice_sample: "声のサンプル", vocabulary: "用語", setting: "設定", model: "モデル", source: "復元元",
};

const progressLabels: Record<string, string> = {
  copying_salvage_database: "元の会議データを一時フォルダーにコピーしています。",
  inspecting_salvage_meetings: "会議を検査しています。",
  inspecting_salvage_voices: "登録した声を検査しています。",
  inspecting_salvage_models: "登録モデルを検査しています。",
  copying_salvage_audio: "復元する音声をコピーしています。",
  building_meetings: "新しい保存先に会議データを保存しています。",
  copying_salvage_models: "復元するモデルをコピーしています。",
  verifying_salvage: "復元したデータを読み込めるか確認しています。",
  switching_salvage: "新しい保存先への切替を確定しています。",
};

function WorkProgress({ state }: { state: Status | undefined }) {
  const progress = state?.progress;
  const label = (progress && progressLabels[progress.stage])
    ?? (state?.phase === "building" ? "新しい保存先にデータを保存しています。"
      : state?.phase === "inspecting" ? "元データのコピーを検査しています。" : "データ復元の状態を確認しています。");
  const amount = (value: number) => progress?.unit === "bytes" ? capacity(value) : `${value.toLocaleString("ja-JP")} 件`;
  return <div>
    <p role="status">{label}</p>
    {progress && progress.total != null && progress.total > 0 && <>
      <progress aria-label={label} max={progress.total} value={progress.completed} />
      <span> {amount(progress.completed)} / {amount(progress.total)}</span>
    </>}
    {progress && progress.total == null && progress.completed > 0 && <p>確認済み {amount(progress.completed)}</p>}
  </div>;
}

function Items({ items, completed = false }: { items: Item[]; completed?: boolean }) {
  return <div className="salvage-inventory"><table>
    <thead><tr><th scope="col">種類</th><th scope="col">対象</th><th scope="col">状態</th><th scope="col">詳細</th></tr></thead>
    <tbody>{items.map((item, index) => <tr key={JSON.stringify([item.kind, item.sourceId, index])}>
      <td>{itemKinds[item.kind] ?? item.kind}</td><td>{item.label}</td>
      <td>{completed && item.status === "recoverable" ? "引継ぎ済み" : completed && item.status === "partial" ? "一部引継ぎ済み" : itemStates[item.status]}</td>
      <td>{item.reason && displayNativePathsInText(item.reason)}</td>
    </tr>)}</tbody>
  </table></div>;
}

function ResultDetails({ result }: { result: Result }) {
  return <>
    <p>会議 {result.meetingCount} 件、登録した人物 {result.voicePersonCount} 人、モデル {result.modelCount} 件を引き継ぎました。</p>
    <dl className="salvage-paths">
      <dt>元の会議データ</dt><dd>{displayNativePath(result.sourceRoot)}</dd>
      {result.sourceModels && <><dt>元のモデル</dt><dd>{displayNativePath(result.sourceModels)}</dd></>}
      <dt>新しい会議データ</dt><dd>{displayNativePath(result.destinationRoot)}</dd>
      <dt>新しいモデル</dt><dd>{displayNativePath(result.destinationModels)}</dd>
    </dl>
    <Items items={result.items} completed />
  </>;
}

function capacity(bytes: number) {
  const unit = bytes >= 1024 ** 3 ? "GB" : "MB";
  return `${(bytes / (unit === "GB" ? 1024 ** 3 : 1024 ** 2)).toLocaleString("ja-JP", { maximumFractionDigits: 2 })} ${unit}`;
}

export function SalvageEntry({
  mode, reasons = [], disabled = false, dedicated = false,
}: {
  mode: "ready" | "blocked";
  reasons?: string[];
  disabled?: boolean;
  dedicated?: boolean;
}) {
  const [open, setOpen] = useState(dedicated);
  const [started, setStarted] = useState(dedicated);
  const [editExport, setEditExport] = useState<EditExport | null>(null);
  const [busy, setBusy] = useState(false);
  const [waitingForRestart, setWaitingForRestart] = useState(false);
  const [state, setState] = useState<Status>();
  const [error, setError] = useState("");
  const [rechecked, setRechecked] = useState<string[]>();
  const [salvageRevoked, setSalvageRevoked] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [released, setReleased] = useState(false);
  const inFlight = useRef(false);
  const active = useRef(false);
  const opener = useRef<HTMLButtonElement>(null);
  const selection = state?.selection;
  const result = selection?.result;
  const preview = selection?.preview;
  const bytesAvailable = selection?.availableBytes;

  async function run(action: () => Promise<void>) {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError("");
    try { await action(); }
    catch (cause) { if (active.current) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally {
      inFlight.current = false;
      if (active.current) {
        setBusy(false);
        setCancelling(false);
      }
    }
  }

  async function readNewState(action: () => Promise<Status>) {
    const next = await action();
    if (active.current) {
      setState(next);
      setError(next.error ?? "");
    }
  }

  useEffect(() => {
    active.current = true;
    return () => { active.current = false; };
  }, []);

  useEffect(() => {
    if (mode !== "blocked" || !open || !started || released) return;
    void run(() => readNewState(() => invoke<Status>("open_salvage")));
  }, [mode, open, started, released]);

  useEffect(() => {
    if (mode !== "blocked" || !open || !started || !busy) return;
    let current = true;
    const timer = window.setInterval(() => {
      void invoke<Status>("get_salvage_state").then(next => {
        if (current && inFlight.current && next.phase !== "idle") {
          setState(previous => ({ ...next, selection: previous?.selection ?? next.selection }));
        }
      }, () => {});
    }, 400);
    return () => { current = false; window.clearInterval(timer); };
  }, [mode, open, started, busy]);

  const dismiss = () => {
    if (busy || waitingForRestart) return;
    if (mode === "blocked" && started) {
      void run(async () => {
        if (!released) {
          await invoke("leave_salvage");
          setReleased(true);
        }
        if (dedicated || result) {
          await invoke("retry_startup");
          setWaitingForRestart(true);
        } else { setStarted(false); setOpen(false); }
      });
    } else setOpen(false);
  };

  async function recheckReadySalvage() {
    try {
      const confirmed = checkedStorageWriteState(await invoke("get_storage_write_state"));
      setRechecked(confirmed.salvageAvailable ? confirmed.reasons : []);
      setSalvageRevoked(!confirmed.salvageAvailable);
      return confirmed.salvageAvailable;
    } catch (cause) {
      setSalvageRevoked(true);
      throw cause;
    }
  }

  const recheck = () => void run(async () => {
    if (mode === "blocked") {
      setRechecked(await invoke<string[]>("recheck_salvage_environment"));
    } else {
      await recheckReadySalvage();
    }
  });

  const switchToSalvage = () => void run(async () => {
    if (!await recheckReadySalvage()) return;
    if (editExport?.status !== "saved") {
      try { await flushPendingEdits(); }
      catch {
        setEditExport({ status: "failed", message: "編集中の内容を保存できませんでした。保存をやり直すか、未保存の入力を別のファイルに保存してください。" });
        return;
      }
    }
    const [importing, data] = await Promise.all([
      invoke<{ token: string | null; busy: boolean }>("get_audio_import"),
      invoke<{ token: string | null; busy: boolean }>("get_data_operation"),
    ]);
    if (importing.busy || data.busy) throw Error("進行中の取り込みとデータ処理が完了してから復元を開始してください。");
    if (!await recheckReadySalvage()) return;
    if (editExport?.status === "saved") {
      let latest: string | null;
      try { latest = pendingEditsSnapshot(); }
      catch { latest = null; }
      if (latest !== editExport.contents) {
        setEditExport({ status: "failed", message: "今の入力がファイルに保存されていることを確認できません。もう一度保存してください。" });
        return;
      }
    }
    await invoke("finish_main_close", { quit: true, salvage: true, expected: { importToken: importing.token, dataToken: data.token } });
    setWaitingForRestart(true);
  });

  const exportEdits = () => void run(async () => {
    try {
      const contents = pendingEditsSnapshot();
      if (contents === null) {
        setEditExport(null);
        return;
      }
      const saved = await invoke<boolean>("export_salvage_edits", { contents });
      if (saved === true) setEditExport({ status: "saved", contents });
      else if (saved === false) setEditExport({ status: "cancelled" });
      else throw Error("未保存の入力がファイルに保存されたことを確認できません。");
    } catch (cause) {
      setEditExport({ status: "failed", message: cause instanceof Error ? cause.message : String(cause) });
    }
  });

  function cancelWork() {
    if (cancelling) return;
    setCancelling(true);
    void invoke("cancel_salvage_work").catch(cause => {
      setCancelling(false);
      setError(String(cause));
    });
  }

  if (!isTauri()) return null;
  return <>
    {!dedicated && <button ref={opener} type="button" className="secondary" disabled={disabled} onClick={() => {
      setReleased(false);
      setEditExport(null);
      setRechecked(undefined);
      setSalvageRevoked(false);
      setError("");
      setOpen(true);
    }}>データ復元</button>}
    <CommonModal portal open={open} blocked={busy || waitingForRestart} onDismiss={dismiss} restoreTarget={() => opener.current} dismissLabel="閉じる" dismissText="データ復元を閉じる"
      className={mode === "blocked" && started ? "salvage-wide-modal" : undefined}
      aria-labelledby="salvage-title">
      <section className="salvage-dialog">
        <h2 id="salvage-title">{result ? "データ復元が完了しました" : "データ復元"}</h2>
        {mode === "ready" || !started ? <>
          <p>保存先が接続されていること、空き容量があること、現在のアカウントでアクセスできることを確認してください。</p>
          {!salvageRevoked && !!(rechecked ?? reasons).length && <ul>{[...new Set(rechecked ?? reasons)].map(reason => <li key={reason}>{displayNativePathsInText(reason)}</li>)}</ul>}
          {!salvageRevoked && rechecked && <p role="status">{mode === "blocked" ? "保存先フォルダーの再確認を行いました。" : "保存先の状態を再確認しました。"}</p>}
          {salvageRevoked ? <p role="status">現在の状態ではデータ復元を利用できません。保存先の状態を確認してください。</p>
            : <p>問題が解消しない場合は、読み取れるデータを新しい保存先に復元できます。元データは保持します。</p>}
          {editExport?.status === "failed" && <p className="salvage-edit-error" role="alert">{displayNativePathsInText(editExport.message)}</p>}
          {editExport?.status === "cancelled" && <p role="status">ファイルへの保存を取り消しました。未保存の入力は編集画面に残っています。</p>}
          {editExport?.status === "saved" && <p role="status">未保存の入力を別のファイルに保存しました。</p>}
          <div className="salvage-actions">
            <button type="button" className="secondary" disabled={busy || waitingForRestart} onClick={recheck}>再確認</button>
            {!salvageRevoked && editExport && editExport.status !== "saved" && <button type="button" className="secondary" disabled={busy || waitingForRestart} onClick={exportEdits}>未保存の入力を別のファイルに保存</button>}
            {!salvageRevoked && <button type="button" disabled={disabled || busy || waitingForRestart} onClick={mode === "ready" ? switchToSalvage : () => setStarted(true)}>
              {mode === "ready" ? "再起動して復元に進む" : "復元内容の確認に進む"}
            </button>}
          </div>
        </> : result ? <>
          <ResultDetails result={result} />
          <button type="button" disabled={busy || waitingForRestart} onClick={dismiss}>再起動して新しい保存先を開く</button>
        </> : <>
          <p>読み取れたデータを新しい保存先に復元します。元データは保持します。</p>
          <dl className="salvage-paths">
            <dt>復元元</dt><dd>{selection?.sourceRoot ? displayNativePath(selection.sourceRoot) : "未確認"}</dd>
            <dt>復元先</dt><dd>{selection?.destinationRoot ? displayNativePath(selection.destinationRoot) : "未選択"}</dd>
            {bytesAvailable != null && <><dt>選択先の空き容量</dt><dd>{capacity(bytesAvailable)}</dd></>}
          </dl>
          {preview && <>
            <p>{preview.meetingTotal === null ? "元の会議総数を確認できませんでした。" : `元の会議 ${preview.meetingTotal} 件を確認しました。`} 復元に必要な容量は {capacity(preview.requiredBytes)} です。</p>
            <Items items={preview.items} />
            {preview.recoverableCount === 0 && <p role="status">復元できるデータを確認できませんでした。</p>}
            {bytesAvailable != null && bytesAvailable < preview.requiredBytes && <p role="status">選択先の空き容量が不足しています。</p>}
          </>}
          {busy && <WorkProgress state={state} />}
          <div className="salvage-actions">
            {selection?.sourceSelectable && <button type="button" className="secondary" disabled={busy || released} onClick={() => void run(() => readNewState(() => invoke<Status>("choose_salvage_folder", { source: true })))}>元のフォルダーを選択</button>}
            {selection && <button type="button" className="secondary" disabled={busy || released} onClick={() => void run(() => readNewState(() => invoke<Status>("choose_salvage_folder", { source: false })))}>{selection.destinationRoot ? "復元先を変更" : "復元先の親フォルダーを選択"}</button>}
            {!selection && !!error && <button type="button" className="secondary" disabled={busy || released} onClick={() => void run(() => readNewState(() => invoke<Status>("open_salvage")))}>再確認</button>}
            {!!selection?.sourceRoot && !preview && <button type="button" disabled={busy || released || !selection.destinationRoot} onClick={() => void run(() => readNewState(() => invoke<Status>("inspect_salvage")))}>復元できる内容を確認</button>}
            {preview && bytesAvailable != null && bytesAvailable < preview.requiredBytes && <button type="button" className="secondary" disabled={busy || released}
              onClick={() => void run(() => readNewState(() => invoke<Status>("inspect_salvage")))}>空き容量を再確認</button>}
            {preview && <button type="button" disabled={busy || released || preview.recoverableCount === 0 || (bytesAvailable != null && bytesAvailable < preview.requiredBytes)}
              onClick={() => void run(() => readNewState(() => invoke<Status>("commit_salvage", { operationId: preview.operationId })))}>復元して保存先を切り替える</button>}
            {busy && (state?.phase === "inspecting" || state?.phase === "building") ? <button type="button" className="secondary" disabled={cancelling} onClick={cancelWork}>{cancelling ? "取消を待っています" : "処理を取り消す"}</button>
              : <button type="button" className="secondary" disabled={busy || waitingForRestart} onClick={dismiss}>{dedicated ? "復元を終了して再起動" : "復元を取り消す"}</button>}
          </div>
        </>}
        {waitingForRestart && <p role="status">再起動を待っています。</p>}
        <ErrorNotice scope="salvage" type="operation" active={!!error}>{displayNativePathsInText(error)}</ErrorNotice>
      </section>
    </CommonModal>
  </>;
}

export function StorageRecoveryNotice({ state, disabled, onRecheck, relocationGuidance }: {
  state: StorageWriteState;
  disabled: boolean;
  onRecheck: () => Promise<void>;
  relocationGuidance?: string;
}) {
  const [checking, setChecking] = useState(false);
  const recheck = async () => {
    setChecking(true);
    try { await onRecheck(); }
    finally { setChecking(false); }
  };
  return <section className="workspace-warning meeting-list-notice storage-recovery-notice" aria-label="保存先の警告">
    <p role="status">{state.salvageAvailable ? "保存先は閲覧専用です。会議を閲覧するか、新しい保存先へデータを復元できます。" : "保存先は閲覧専用です。会議を閲覧できます。"}</p>
    {state.salvageAvailable ? <SalvageEntry mode="ready" reasons={state.reasons} disabled={disabled} /> : <>
      <button type="button" className="secondary" disabled={checking} onClick={() => void recheck()}>保存先の状態を再確認</button>
      {relocationGuidance && <p>{relocationGuidance}</p>}
      {checking && <p role="status">保存先の状態を確認しています。</p>}
    </>}
  </section>;
}

function checkedCleanup(result: Result, value: unknown): SalvageSourceCleanup {
  if (!validateSalvageSourceCleanup(value) || !nativeAbsolutePath(value.sourceRoot) || value.sourceRoot !== result.sourceRoot
    || (value.retainedAudioFiles ?? 0) > value.audioFiles
    || (value.retainedAudioFiles ?? 0) > 0 && value.state === "available") {
    throw Error("復元元のファイルを確認できませんでした。削除状態を再確認してください。");
  }
  return value;
}

async function readCleanup(result: Result): Promise<SalvageSourceCleanup | null> {
  const data = await invoke<unknown>("get_salvage_source_cleanup", { operationId: result.operationId });
  return data === null ? null : checkedCleanup(result, data);
}

function sameCleanup(current: SalvageSourceCleanup | null, reviewed: SalvageSourceCleanup): boolean {
  return !!current && current.sourceRoot === reviewed.sourceRoot && current.state === reviewed.state
    && current.databaseFiles === reviewed.databaseFiles && current.audioFiles === reviewed.audioFiles
    && (current.retainedAudioFiles ?? 0) === (reviewed.retainedAudioFiles ?? 0);
}

function cleanupErrorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

function CleanupConfirmation({ review, progress, unavailableReason, onCancel, onDelete }: {
  review: SalvageSourceCleanup;
  progress: "checking" | "deleting" | null;
  unavailableReason: string | null;
  onCancel: () => void;
  onDelete: () => void;
}) {
  return <section className="salvage-dialog salvage-source-confirmation">
    <h2 id="salvage-cleanup-title" tabIndex={-1}>元データの削除を確認</h2>
    <p>元の保存先から、下記のファイルを削除します。復元できなかったデータも含まれる場合があり、削除すると元に戻せません。</p>
    <p>元のモデルや、アプリが保存したか確認できないファイルは残ります。復元後に移動・変更された音声も削除対象から外れます。</p>
    <dl className="salvage-paths">
      <dt>元の保存先</dt><dd>{displayNativePath(review.sourceRoot)}</dd>
      <dt>元の会議データと関連ファイル</dt><dd>{review.databaseFiles.toLocaleString("ja-JP")} 件</dd>
      <dt>復元済みの音声ファイル</dt><dd>{review.audioFiles.toLocaleString("ja-JP")} 件</dd>
    </dl>
    {progress && <p role="status">{progress === "checking" ? "削除対象を再確認しています。" : "元データを削除しています。"}</p>}
    {!progress && unavailableReason && <p role="status">{displayNativePathsInText(unavailableReason)}</p>}
    <div className="salvage-actions">
      <button type="button" className="secondary" disabled={!!progress} onClick={onCancel}>キャンセル</button>
      <button type="button" className="stop-button" disabled={!!progress || !!unavailableReason} onClick={onDelete}>元データを削除する</button>
    </div>
  </section>;
}

export function SalvageHistory({ cleanupUnavailableReason = null }: { cleanupUnavailableReason?: string | null }) {
  const [result, setResult] = useState<Result | null>();
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState("");
  const [cleanup, setCleanup] = useState<SalvageSourceCleanup | null>();
  const [checkingCleanup, setCheckingCleanup] = useState(false);
  const [cleanupError, setCleanupError] = useState("");
  const [review, setReview] = useState<SalvageSourceCleanup | null>(null);
  const [deleteProgress, setDeleteProgress] = useState<"checking" | "deleting" | null>(null);
  const historyRequest = useRef(0);
  const deletingNow = useRef(false);
  const unavailable = useRef(cleanupUnavailableReason);
  unavailable.current = cleanupUnavailableReason;
  const historyOpener = useRef<HTMLButtonElement>(null);
  const cleanupOpener = useRef<HTMLButtonElement>(null);
  useEffect(() => () => { ++historyRequest.current; }, []);

  async function checkCleanup(value: Result) {
    const request = historyRequest.current;
    setCheckingCleanup(true);
    setCleanupError("");
    try {
      const next = await readCleanup(value);
      if (request === historyRequest.current) setCleanup(next);
    } catch (cause) {
      if (request === historyRequest.current) {
        setCleanup(undefined);
        setCleanupError(`元データの削除状態を確認できませんでした。${cleanupErrorMessage(cause)}`);
      }
    } finally {
      if (request === historyRequest.current) setCheckingCleanup(false);
    }
  }

  async function show() {
    const request = ++historyRequest.current;
    setOpen(true);
    setLoading(true);
    setResult(undefined);
    setCleanup(undefined);
    setCheckingCleanup(false);
    setCleanupError("");
    setError("");
    try {
      const next = await invoke<Result | null>("get_salvage_result");
      if (request !== historyRequest.current) return;
      setResult(next);
      if (next) void checkCleanup(next);
    } catch (cause) {
      if (request === historyRequest.current) setError(String(cause));
    } finally {
      if (request === historyRequest.current) setLoading(false);
    }
  }

  function close() {
    ++historyRequest.current;
    setOpen(false);
  }

  function closeReview() {
    setReview(null);
  }

  async function deleteSource() {
    if (!open || !result || !review || deletingNow.current || unavailable.current) return;
    const request = historyRequest.current;
    deletingNow.current = true;
    setDeleteProgress("checking");
    setCleanupError("");
    let dispatched = false;
    try {
      const current = await readCleanup(result);
      if (request !== historyRequest.current) return;
      setCleanup(current);
      if (unavailable.current) return;
      if (!sameCleanup(current, review)) {
        if (current?.state !== "completed") setCleanupError("削除対象の状態が変わりました。現在の対象を確認してください。");
        return;
      }
      dispatched = true;
      setDeleteProgress("deleting");
      const reply = await invoke<unknown>("delete_salvage_source", { operationId: result.operationId });
      const next = checkedCleanup(result, reply);
      if (next.state === "available") throw Error("削除状態を再確認してください。");
      if (request === historyRequest.current) setCleanup(next);
    } catch (cause) {
      if (request !== historyRequest.current) return;
      setCleanup(undefined);
      setCleanupError(`${dispatched ? "元データの削除結果を確認できませんでした。" : "削除対象を確認できませんでした。"}${cleanupErrorMessage(cause)}`);
      if (dispatched) {
        const recorded = await readCleanup(result).catch(() => undefined);
        // A retained preview alone cannot establish that a lost submission
        // did not start. Re-read explicitly before offering a new confirmation.
        if (request === historyRequest.current && recorded && recorded.state !== "available") {
          setCleanup(recorded);
          setCleanupError(recorded.state === "completed" ? "" : cleanupErrorMessage(cause));
        }
      }
    } finally {
      deletingNow.current = false;
      if (request === historyRequest.current) { setDeleteProgress(null); closeReview(); }
    }
  }

  if (!isTauri()) return null;
  const canReview = cleanup && cleanup.state !== "completed" && (cleanup.databaseFiles > 0 || cleanup.audioFiles > 0);
  const retainedAudioFiles = cleanup?.retainedAudioFiles ?? 0;
  const reviewView = review && {
    key: "salvage-source-cleanup",
    labelledBy: "salvage-cleanup-title",
    blocked: !!deleteProgress,
    onDismiss: closeReview,
    dismissLabel: "キャンセル" as const,
    dismissText: "削除をキャンセル",
    restoreTarget: () => cleanupOpener.current,
    content: <CleanupConfirmation review={review} progress={deleteProgress} unavailableReason={cleanupUnavailableReason} onCancel={closeReview} onDelete={() => void deleteSource()} />,
  };
  return <>
    <button ref={historyOpener} type="button" className="secondary" onClick={() => void show()}>復元結果と元データの保存先を確認</button>
    <CommonModal portal open={open} onDismiss={close} restoreTarget={() => historyOpener.current} dismissLabel="閉じる" dismissText="復元結果を閉じる"
      replacement={reviewView} className="salvage-wide-modal" aria-labelledby="salvage-history-title">
      <section className="salvage-dialog">
        <h2 id="salvage-history-title">データ復元結果</h2>
        {loading ? <p role="status">読み込み中です。</p> : result ? <ResultDetails result={result} /> : result === null ? <p>この保存先の復元記録はありません。</p> : null}
        {result && <section className="salvage-source-cleanup" aria-label="元データの削除">
          <h3>元データの削除</h3>
          {checkingCleanup && <p role="status">元データの削除状態を確認しています。</p>}
          {cleanup?.state === "completed" && <p role="status">
            確認した元データの削除が完了しました。
            {retainedAudioFiles > 0 && `元のファイルと確認できなかった音声 ${retainedAudioFiles.toLocaleString("ja-JP")} 件は削除しませんでした。`}
          </p>}
          {cleanup?.state === "interrupted" && <p role="status">元データの削除が中断しました。{canReview ? "現在の対象を確認して削除を続けられます。" : "現在の削除対象を確認できません。"}</p>}
          {(cleanup === null || cleanup?.state === "available" && !canReview) && <p>元の保存先で削除できるファイルを確認できませんでした。</p>}
          {cleanupError && <p className="confirmation-error" role="alert">{displayNativePathsInText(cleanupError)}</p>}
          {cleanup === undefined && cleanupError && <button type="button" className="secondary" disabled={checkingCleanup} onClick={() => void checkCleanup(result)}>削除状態を再確認</button>}
          {canReview && cleanupUnavailableReason && <p role="status">{displayNativePathsInText(cleanupUnavailableReason)}</p>}
          {canReview && <button ref={cleanupOpener} type="button" className="secondary" disabled={checkingCleanup || !!cleanupUnavailableReason} onClick={() => {
            cleanupOpener.current?.focus({ preventScroll: true });
            setReview(cleanup);
          }}>{cleanup.state === "interrupted" ? "元データの削除を続ける…" : "元データを削除…"}</button>}
        </section>}
        <ErrorNotice scope="salvage-history" type="read" active={!!error}>{displayNativePathsInText(error)}</ErrorNotice>
      </section>
    </CommonModal>
  </>;
}
