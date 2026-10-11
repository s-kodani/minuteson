import { CommonModal } from "./common-modal";
import { IconButton } from "./action-icon";
import { IssueNotice, IssueStatus, ToastNotice } from "./toast";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useId, useRef, useState } from "react";
import type { AnalysisJob } from "./analysis-state";
import { InspectionAttempt, type ModelPlacementEntry } from "./model-inspection-state";
import { assertJobModelPreview, assertJobModelResult, canRebindJob, rebindingEntries, ReviewedModelRetry, type JobModelPreview, type RebindingJob } from "./model-rebinding-state";

const unresolved = new Map<string, ReviewedModelRetry>();
type Receipt = { job: RebindingJob | null; active: boolean; cancelled: boolean };

export function JobModelRebinding({ meetingId, job, jobs, locked, recordingActive, withSavedEdits, refresh }: {
  meetingId: string; job: AnalysisJob; jobs: AnalysisJob[]; locked: boolean; recordingActive: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>; refresh: () => Promise<void>;
}) {
  const key = `${meetingId}/${job.id}`, titleId = useId();
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<ModelPlacementEntry[]>([]);
  const [selected, setSelected] = useState("");
  const [review, setReview] = useState<ReviewedModelRetry | null>(null);
  const [busy, setBusy] = useState<"catalog" | "preview" | "apply" | "receipt" | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [noticeEvent, setNoticeEvent] = useState("");
  const [noticeLevel, setNoticeLevel] = useState<"info" | "warning">("info");
  const dialog = useRef<HTMLDialogElement>(null), opener = useRef<HTMLElement | null>(null);
  const mounted = useRef(true), active = useRef<{ kind: string; token: string } | null>(null);
  const savingEdits = useRef(false);
  const previewAttempt = useRef(new InspectionAttempt());
  const latest = useRef({ locked, recordingActive, job, jobs });
  latest.current = { locked, recordingActive, job, jobs };
  const restricted = () => (latest.current.locked && !savingEdits.current) || latest.current.recordingActive;
  const saved = async (action: () => Promise<void>) => {
    // The parent's save guard intentionally locks the whole screen and catches
    // errors. Distinguish that owned lock and propagate failure to this review.
    savingEdits.current = true;
    let ran = false, failed = false, failure: unknown;
    try {
      await withSavedEdits(async () => { ran = true; try { await action(); } catch (cause) { failed = true; failure = cause; } });
      if (!ran) throw Error("編集の保存を完了できませんでした。入力を確認し、保存を再試行してください。受付処理は開始していません。");
      if (failed) throw failure;
    } finally { savingEdits.current = false; }
  };
  const cancel = async () => {
    const operation = active.current;
    if (!operation || !["preview", "apply"].includes(operation.kind) || cancelling) return;
    previewAttempt.current.cancel();
    const pending = unresolved.get(key);
    if (pending?.operationId === operation.token) pending.cancelled = true;
    setCancelling(true); setError("");
    try { await invoke("cancel_model_inspection", { token: operation.token }); }
    catch (cause) { if (mounted.current) setError(`取消要求を確認できません。処理終了後に結果を確認してください: ${String(cause)}`); }
  };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      previewAttempt.current.cancel();
      const operation = active.current;
      const pending = unresolved.get(key);
      const token = operation?.kind === "preview" ? operation.token : pending?.operationId;
      if (pending) pending.cancelled = true;
      if (token && isTauri()) void invoke("cancel_model_inspection", { token }).catch(() => {});
    };
  }, [key]);
  useEffect(() => {
    if (!open || !restricted()) return;
    if (active.current && ["preview", "apply"].includes(active.current.kind)) void cancel();
    else if (!review?.sent) { setReview(null); setError("録音・画面の状態が変わりました。処理できる状態に戻ってから再確認してください。"); }
  }, [locked, recordingActive, open]);
  const finish = (token: string) => {
    if (active.current?.token !== token) return;
    active.current = null;
    if (mounted.current) { setBusy(null); setCancelling(false); }
  };
  const load = async () => {
    if (active.current || !isTauri()) return;
    const token = crypto.randomUUID(); active.current = { kind: "catalog", token }; setBusy("catalog"); setError("");
    try {
      const values = rebindingEntries(await invoke<ModelPlacementEntry[]>("get_model_placements"), job.kind);
      if (mounted.current) { setEntries(values); setSelected(""); setReview(null); }
    } catch (cause) { if (mounted.current) { setEntries([]); setError(String(cause)); } }
    finally { finish(token); }
  };
  const show = (element: HTMLElement) => {
    if (active.current || !isTauri()) return;
    const pending = unresolved.get(key);
    if (!pending && (restricted() || !canRebindJob(job, jobs))) return;
    opener.current = element; setOpen(true); setError(""); setNotice(""); setReview(pending ?? null);
    if (!pending) void load();
  };
  const prepare = async () => {
    if (active.current || restricted() || !canRebindJob(latest.current.job, latest.current.jobs) || review?.sent) return;
    const entry = entries.find(e => e.packageId === selected);
    if (!entry) return;
    const token = crypto.randomUUID(); active.current = { kind: "preview", token }; previewAttempt.current.begin(token);
    setBusy("preview"); setCancelling(false); setError(""); setReview(null);
    try {
      await saved(async () => {
        if (!mounted.current || !previewAttempt.current.accepts(token) || restricted()) return;
        const p = await invoke<JobModelPreview>("preview_job_model_rebinding", {
          token, meetingId, jobId: job.id, expectedSpecHash: job.specHash,
          packageId: entry.packageId, expectedReviewHash: entry.reviewHash, checkOperationId: entry.compatibility!.operationId,
        });
        if (!mounted.current || !previewAttempt.current.accepts(token) || restricted()) return;
        assertJobModelPreview(p, meetingId, latest.current.job, entry);
        setReview(new ReviewedModelRetry(p, crypto.randomUUID()));
      });
    } catch (cause) { if (mounted.current && previewAttempt.current.accepts(token)) setError(String(cause)); }
    finally { previewAttempt.current.finish(token); finish(token); }
  };
  const accept = (result: RebindingJob, attempt: ReviewedModelRetry) => {
    assertJobModelResult(result, attempt.operationId, attempt.preview);
    if (unresolved.get(key) === attempt) unresolved.delete(key);
    if (mounted.current) { setReview(null); setOpen(false); setError(""); setNoticeEvent(attempt.operationId); setNoticeLevel(attempt.cancelled ? "warning" : "info"); setNotice(`${attempt.cancelled ? "取消要求より先に、" : ""}同条件の再試行を受付済みです（試行 ${result.attempt}、ジョブ ${result.id.slice(0, 8)}）。解析の完了ではありません。現在版・編集・今後の解析設定は変更していません。`); }
  };
  const receipt = async (attempt: ReviewedModelRetry) => {
    const value = await invoke<Receipt>("get_job_model_rebinding_result", attempt.receipt());
    if (!value || typeof value.active !== "boolean" || typeof value.cancelled !== "boolean" || !(value.job === null || typeof value.job === "object")) throw Error("結果確認の応答が不正です。操作IDを保持して再確認してください。");
    if (value.job) accept(value.job, attempt);
    else if (value.cancelled && !value.active) {
      if (unresolved.get(key) === attempt) unresolved.delete(key);
      if (mounted.current) { setReview(null); setSelected(""); setOpen(false); setError(""); setNoticeEvent(attempt.operationId); setNoticeLevel("info"); setNotice("再試行の受付取消と処理終了を確認しました。この操作では新しいジョブを受け付けていません。現在版・編集は保持しています。"); }
    } else if (mounted.current) setError(value.active ? "受付処理はまだ終了していません。終了後に同じ操作IDで結果を確認してください。" : "この操作の受付記録はまだ確認できません。結果確認は新しいジョブを作りません。同じ操作で受付を再試行できます。確認内容は変更できません。");
  };
  const checkResult = async () => {
    if (active.current || !review?.sent) return;
    const token = crypto.randomUUID(); active.current = { kind: "receipt", token }; setBusy("receipt"); setError("");
    try { await receipt(review); }
    catch (cause) { if (mounted.current) setError(`結果を確認できません。操作IDを保持しています: ${String(cause)}`); }
    finally { finish(token); if (mounted.current) await refresh(); }
  };
  const cancelUnknown = async () => {
    if (active.current || !review?.sent) return;
    const attempt = review, token = crypto.randomUUID();
    active.current = { kind: "receipt", token }; setBusy("receipt"); setError("");
    attempt.cancelled = true;
    try {
      await invoke("cancel_model_inspection", { token: attempt.operationId });
      await receipt(attempt);
    } catch (cause) { if (mounted.current) setError(`取消・結果確認を確定できません。操作IDを保持しています: ${String(cause)}`); }
    finally { finish(token); if (mounted.current) await refresh(); }
  };
  const apply = async () => {
    if (active.current || restricted() || !review || review.cancelled) return;
    const attempt = review, token = attempt.operationId;
    active.current = { kind: "apply", token }; setBusy("apply"); setCancelling(false); setError("");
    try {
      await saved(async () => {
        if (!mounted.current || restricted() || attempt.cancelled) return;
        const request = attempt.request(); unresolved.set(key, attempt);
        const result = await invoke<RebindingJob>("apply_job_model_rebinding", request);
        // A commit may have won a cancellation race. Never suppress its receipt.
        accept(result, attempt);
      });
    } catch (cause) {
      if (mounted.current) setError(attempt.sent ? `受付結果を確定できません。操作IDを保持して結果確認してください: ${String(cause)}` : String(cause));
    } finally {
      if (attempt.sent && attempt.cancelled && unresolved.has(key)) {
        try { await receipt(attempt); } catch (cause) { if (mounted.current) setError(`取消後の結果を確認できません: ${String(cause)}`); }
      }
      finish(token); if (mounted.current) await refresh();
    }
  };
  const pending = unresolved.has(key);
  const eligible = canRebindJob(job, jobs);
  if (!open && !pending && !eligible && !notice) return null;
  return <>
    {(eligible || pending) && <button type="button" className="small-secondary" disabled={!!busy || (!pending && (locked || recordingActive))} onClick={e => show(e.currentTarget)}>{pending ? "モデル再試行の受付結果を確認" : "再配置モデルで同条件の再試行を確認"}</button>}
    <ToastNotice scope={key} type="model-rebinding-result" message={notice} eventId={noticeEvent} level={noticeLevel} important={noticeLevel === "warning"} onClose={noticeLevel === "info" ? () => setNotice("") : undefined} />
    {noticeLevel === "warning" && <IssueStatus label="取消後のモデル再試行受付" message={notice} />}
    <IssueNotice scope={key} type="model-rebinding-operation" label="同条件モデル再試行" message={error} toastMessage="同条件モデル再試行を確定できません。確認情報を保持しています" />
    {open && <CommonModal ref={dialog} open blocked={!!busy} onDismiss={() => { if (!active.current) setOpen(false); }} restoreTarget={() => opener.current} className="analysis-dialog model-rebinding-dialog" aria-labelledby={titleId}>
      <div className="card-head"><h2 id={titleId} tabIndex={-1}>再配置モデルによる同条件の再試行</h2><IconButton label="閉じる" icon="close" disabled={!!busy} data-dialog-cancel onClick={() => setOpen(false)} /></div>
      <p>元のジョブの音声・入力版・用語・人数・数値条件と保存済み途中結果を固定し、同じ構成・同じバイトのモデルだけを再照合します。モデル変更や設定変更による新規解析とは別の操作です。</p>
      {!review?.sent && <>
        <p>先に「モデルの手動検査」で配置と明示的なCPU互換性確認を完了してください。同じエンジンの成功記録だけを表示します。元モデルとの一致は下の確認時に照合します。自動選択・自動試行は行いません。</p>
        <label className="prompt-label">再配置先モデル<select value={selected} disabled={!!busy || locked || recordingActive} onChange={e => { setSelected(e.target.value); setReview(null); setError(""); }}><option value="">明示的に選択してください</option>{entries.map(e => <option key={e.packageId} value={e.packageId}>{e.modelName} {e.modelVersion} · {e.packageId.slice(0, 8)}</option>)}</select></label>
        {!entries.length && !busy && <p>この工程でCPU互換性確認済みの配置記録はありません。</p>}
        <div className="analysis-actions"><IconButton label="配置記録を再読込" icon="refresh" type="button" className="small-secondary" disabled={!!busy || locked || recordingActive} onClick={() => void load()} /><button type="button" disabled={!!busy || !selected || locked || recordingActive} onClick={() => void prepare()}>元の固定条件と配置先を照合</button></div>
      </>}
      {review && <>
        <h3>変更しない条件</h3>
        <p>元ジョブ: <code>{review.preview.source.id}</code> · 試行 {review.preview.source.attempt} → {review.preview.source.attempt + 1}<br />保存済み {review.preview.source.completedUnits} / {review.preview.source.totalUnits} 単位を引き継ぎ、未完了分だけ処理します。</p>
        <p>固定条件SHA-256: <code>{review.preview.source.specHash}</code><br />入力SHA-256: <code>{review.preview.source.inputHash}</code><br />途中結果SHA-256: <code>{review.preview.checkpointHash}</code></p>
        <pre>{JSON.stringify(review.preview.logicalConditions, null, 2)}</pre>
        <h3>再配置先と照合証拠</h3>
        <p>{review.preview.binding.package.manifest.modelName} {review.preview.binding.package.manifest.modelVersion} · {review.preview.binding.package.manifest.engine} {review.preview.binding.package.manifest.engineVersion}<br />対象: {review.preview.binding.package.manifest.target} · ライセンス申告: {review.preview.binding.package.manifest.licenseRef}<br />パッケージ: <code>{review.preview.binding.package.packageId}</code><br />CPU確認操作: <code>{review.preview.binding.compatibility.operationId}</code></p>
        <details><summary>全依存ファイルの照合結果（{review.preview.binding.files.length}件）</summary><ul>{review.preview.binding.files.map((file, i) => <li key={file.path}><div><strong>{review.preview.binding.package.files[i].role} · {review.preview.binding.package.files[i].relativePath}</strong><code>{file.path}</code><span>{file.size} bytes</span><code>{file.sha256}</code></div></li>)}</ul></details>
        <p>CPUの短い互換性試行は、元のGPU・人数・用語条件、速度や品質の保証ではありません。現在版・編集・今後の解析設定は変更しません。受付後は通常のジョブキューで処理を開始します。</p>
        <p>確認内容SHA-256: <code>{review.preview.reviewHash}</code><br />再試行操作ID: <code>{review.operationId}</code></p>
        <div className="analysis-actions">
          {review.sent && <button type="button" disabled={!!busy} onClick={() => void checkResult()}>同じ操作IDで結果だけ確認</button>}
          {review.sent && <button type="button" className="small-secondary" disabled={!!busy} onClick={() => void cancelUnknown()}>未確定の受付を取り消して確認</button>}
          <button type="button" disabled={!!busy || locked || recordingActive || review.cancelled} onClick={() => void apply()}>{review.sent ? "同じ操作で受付を再試行" : "この固定条件で再試行を開始"}</button>
        </div>
      </>}
      {busy && <p role="status">{cancelling ? "取消要求済み。処理終了と受付結果を待っています。" : ({ catalog: "配置記録を読込中", preview: "固定条件・モデル・途中結果を照合中", apply: "照合を再確認して再試行を受付中", receipt: "受付結果を読み取り確認中" })[busy]}</p>}
      {busy && ["preview", "apply"].includes(busy) && <button type="button" className="small-secondary" disabled={cancelling} onClick={() => void cancel()}>照合・受付処理の取消を要求</button>}
      {review?.sent && <p>応答が未確定でも操作IDと確認内容を保持します。別のモデル・新しい操作IDへ自動的に切り替えません。閉じた後も元ジョブから結果確認できます。</p>}
      {error && <p className="action-error" role="alert">{error}</p>}
    </CommonModal>}
  </>;
}
