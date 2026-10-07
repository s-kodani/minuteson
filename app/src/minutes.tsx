import { invoke, isTauri } from "@tauri-apps/api/core";
import { CommonModal } from "./common-modal";
import { InfoPopover } from "./info-popover";
import { IssueNotice, ToastNotice } from "./toast";
import { useActionConfirmation } from "./confirmation-service";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useId, useRef, useState } from "react";
import { DraftSaveQueue, registerPendingEdits, type VersionedDraft } from "./draft-save";
import { canRetryJob, jobLabel, type AnalysisJob, type VersionState } from "./analysis-state";
import { JobModelRebinding } from "./model-rebinding";
import { assertMinutesReview, assertMinutesVersion, minutesConfigured, minutesExportRequest, exportLabel, exportNoticeLevel, type MinutesExport, type MinutesExportRequest, type MinutesDraft, type MinutesEngine, type MinutesPreferences, type MinutesVersion, type MinutesVersions, type MinutesView } from "./minutes-state";
import { MinutesDocument } from "./minutes-document";
import { MinutesEditor, type EditorGuard } from "./minutes-editor";

type Preferences = VersionedDraft<MinutesPreferences>;
type Job = AnalysisJob & { operationId: string; outputMinutesId: string | null };
type Preview = { summary: MinutesVersion; original: MinutesVersions; view: MinutesView; draft: MinutesDraft | null; current: MinutesView | null; currentDraft: MinutesDraft | null; restoring: boolean };

export function MinutesPanel({ meetingId, meetingTitle, sourceEpoch, recordingActive, locked, withSavedEdits, onHeadChanged }: {
  meetingId: string; meetingTitle: string; sourceEpoch: number; recordingActive: boolean; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  onHeadChanged: () => Promise<void>;
}) {
  const profileSelectId = useId();
  const confirm = useActionConfirmation(locked);
  const [queue] = useState(() => new DraftSaveQueue<MinutesPreferences, Preferences>(
    (_id, expectedLockVersion, payload) => invoke("save_minutes_preferences", { expectedLockVersion, payload }),
    (value) => structuredClone(value),
  ));
  const [settings, setSettings] = useState(() => queue.snapshot());
  const [profile, setProfile] = useState<"standard" | "highAccuracy">("standard");
  const [versions, setVersions] = useState<MinutesVersions | null>(null);
  const [source, setSource] = useState<VersionState | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [current, setCurrent] = useState<MinutesView | null>(null);
  const [currentDraft, setCurrentDraft] = useState<MinutesDraft | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [exports, setExports] = useState<MinutesExport[]>([]);
  const [exportNotice, setExportNotice] = useState("");
  const [exportEvent, setExportEvent] = useState("");
  const [exportState, setExportState] = useState<MinutesExport["state"] | null>(null);
  const pendingExport = useRef<MinutesExportRequest | null>(null);
  const mounted = useRef(true);
  const busyRef = useRef(false);
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const loadedCurrent = useRef("");
  const editorGuard = useRef<EditorGuard | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const operations = useRef(new Map<string, string>());
  const retryOperations = useRef(new Map<string, string>());

  useEffect(() => {
    mounted.current = true;
    const stop = queue.subscribe(() => setSettings(queue.snapshot()));
    const unregister = registerPendingEdits(() => queue.flush());
    if (isTauri()) void invoke<Preferences>("get_minutes_preferences").then((value) => { if (mounted.current) queue.load(value); }).catch((cause: unknown) => { if (mounted.current) setError(String(cause)); });
    return () => { mounted.current = false; stop(); unregister(); };
  }, [queue]);
  useEffect(() => {
    if (!settings.dirty || settings.saving || settings.error) return;
    const timer = window.setTimeout(() => { void queue.flush().catch(() => {}); }, 700);
    return () => window.clearTimeout(timer);
  }, [queue, settings]);
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let inFlight: Promise<void> | null = null;
    let again = false;
    const stops: (() => void)[] = [];
    const refresh = (): Promise<void> => {
      if (disposed) return Promise.resolve();
      if (inFlight) { again = true; return inFlight; }
      inFlight = (async () => {
        do {
          again = false;
          try {
            const [state, analysis, allJobs, exportHistory] = await Promise.all([
              invoke<MinutesVersions>("get_minutes_versions", { meetingId }),
              invoke<VersionState>("get_analysis_versions", { meetingId }),
              invoke<Job[]>("get_analysis_jobs", { meetingId }),
              invoke<MinutesExport[]>("get_minutes_exports", { meetingId }),
            ]);
            if (disposed) return;
            const hash = state.versions.find((v) => v.id === state.currentId)?.contentHash;
            const key = JSON.stringify([state.currentId, hash, state.draft]);
            if (loadedCurrent.current !== key) {
              if (editorGuard.current && !editorGuard.current(state.currentId)) throw new Error("議事録の現在版が変更されています。編集中の内容を保持しています。保存・競合を確認してから再読込してください。");
              const [view, draft] = state.currentId ? await Promise.all([
                invoke<MinutesView | null>("get_minutes_preview", { meetingId, minutesId: state.currentId }),
                invoke<MinutesDraft | null>("get_minutes_draft", { meetingId, minutesId: state.currentId }),
              ]) : [null, null];
              if (disposed) return;
              if (state.currentId && (!view || view.id !== state.currentId || view.contentHash !== hash || draft?.id !== state.draft?.id || draft?.lockVersion !== state.draft?.lockVersion)) throw new Error("議事録の読込中に版・編集が変わりました。再読込してください。");
              setCurrent(view); setCurrentDraft(draft); loadedCurrent.current = key;
            }
            setVersions(state); setSource(analysis); setJobs(allJobs);
            setExports(exportHistory);
            // A known job resolves a lost start response. Subsequent explicit
            // generations may then receive a fresh operation ID.
            for (const [key, operation] of operations.current) if (allJobs.some((j) => j.operationId === operation)) operations.current.delete(key);
            for (const [key, operation] of retryOperations.current) if (allJobs.some((j) => j.operationId === operation)) retryOperations.current.delete(key);
          } catch (cause) { if (!disposed) setError(String(cause)); }
        } while (again && !disposed);
      })().finally(() => { inFlight = null; });
      return inFlight;
    };
    refreshRef.current = refresh;
    for (const event of ["analysis-job-changed", "analysis-version-changed", "minutes-version-changed"]) {
      void listen(event, () => { void refresh(); }).then((stop) => { if (disposed) stop(); else { stops.push(stop); void refresh(); } }).catch((cause: unknown) => { if (!disposed) setError(String(cause)); });
    }
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 3000);
    return () => { disposed = true; for (const stop of stops) stop(); window.clearInterval(timer); };
  }, [meetingId]);
  useEffect(() => { void refreshRef.current(); }, [sourceEpoch]);
  const run = async (action: () => Promise<void>) => {
    if (busyRef.current || locked) return;
    busyRef.current = true; setBusy(true); setError("");
    try { await action(); } catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { busyRef.current = false; if (mounted.current) setBusy(false); }
  };
  const saved = (action: () => Promise<void>) => withSavedEdits(async () => {
    try { await action(); } catch (cause) { if (mounted.current) setError(String(cause)); throw cause; }
  });
  const update = (patch: Partial<MinutesEngine>) => queue.edit((value) => ({ ...value, [profile]: { ...value[profile], ...patch } }));
  const start = () => run(() => saved(async () => {
    const engine = queue.snapshot().payload?.[profile];
    if (!engine || !minutesConfigured(engine)) throw new Error("議事録の実行ファイル・GGUFモデル・入力と出力の上限を確認してください。");
    const analysis = await invoke<VersionState>("get_analysis_versions", { meetingId });
    const version = analysis.versions.find((v) => v.id === analysis.currentId);
    if (!version) throw new Error("採用済みの文字起こしが必要です。");
    const base = { input: { meetingId, expectedAnalysisId: version.id, expectedContentHash: version.contentHash, expectedMeetingLockVersion: analysis.meetingLockVersion, expectedDraft: analysis.draft }, ...engine, profile: profile === "highAccuracy" ? "high_accuracy" : "standard" };
    const key = JSON.stringify(base);
    const operationId = operations.current.get(key) ?? crypto.randomUUID();
    operations.current.set(key, operationId);
    await invoke("start_minutes_generation", { request: { ...base, input: { ...base.input, operationId } } });
    operations.current.delete(key);
    await refreshRef.current();
  }));
  const jobAction = (command: string, jobId: string) => run(async () => { await invoke(command, { jobId }); await refreshRef.current(); });
  const retryJob = (job: Job) => {
    return run(async () => {
      await confirm({ title: "議事録生成の再試行を確認", target: `会議「${meetingTitle}」 · ジョブ ${job.id}`, confirmLabel: "同じ条件で再試行する", message: `この議事録生成を同じ固定条件で再試行します。保存済み ${job.completedUnits} 単位を再照合して引き継ぎます。現在の文字起こし・モデル設定の変更は使用しません。結果は候補として保存し、現在版と編集は変更しません。`, onError: cause => { if (mounted.current) setError(String(cause)); }, action: async () => {
      const operationId = retryOperations.current.get(job.id) ?? crypto.randomUUID();
      retryOperations.current.set(job.id, operationId);
      try {
        await invoke("retry_minutes_generation", { request: { meetingId, jobId: job.id, operationId, expectedSpecHash: job.specHash } });
        retryOperations.current.delete(job.id);
      } finally { await refreshRef.current(); }
      } });
    });
  };
  const cancelJob = (job: Job) => run(async () => { await confirm({ title: "議事録生成の取消を確認", target: `会議「${meetingTitle}」 · ジョブ ${job.id}`, confirmLabel: "生成を取り消す", message: "この議事録生成を取り消します。現在版と編集は保持します。", onError: cause => { if (mounted.current) setError(String(cause)); }, action: async () => { await invoke("cancel_analysis_job", { jobId: job.id }); await refreshRef.current(); } }); });
  const writeMarkdown = () => run(() => saved(async () => {
    setExportNotice("");
    // Inspect an unknown response before allowing a new dialog or file write.
    if (pendingExport.current) {
      const status = await invoke<{ active: boolean; record: MinutesExport | null }>("get_minutes_export_status", { request: pendingExport.current });
      if (status.active) throw new Error("保存ダイアログまたは書き出しが進行中です。完了・取消を待ってください。");
      pendingExport.current = null;
      if (!mounted.current) return;
      setExportEvent(status.record?.id ?? crypto.randomUUID()); setExportState(status.record?.state ?? null);
      setExportNotice(status.record ? `${exportLabel(status.record.state)}: ${status.record.destinationPath}${status.record.errorMessage ? ` — ${status.record.errorMessage}` : ""}` : "前回の出力履歴はありません。保存先を確認し、必要なら改めて書き出してください。");
      await refreshRef.current(); return;
    }
    const state = await invoke<MinutesVersions>("get_minutes_versions", { meetingId });
    pendingExport.current = minutesExportRequest(state, meetingId, crypto.randomUUID());
    const record = await invoke<MinutesExport | null>("export_minutes_markdown", { request: pendingExport.current });
    pendingExport.current = null;
    if (!mounted.current) return;
    setExportEvent(record?.id ?? crypto.randomUUID()); setExportState(record?.state ?? null);
    setExportNotice(record ? `${exportLabel(record.state)}: ${record.destinationPath}${record.errorMessage ? ` — ${record.errorMessage}` : ""}` : "書き出しを取り消しました。ローカルの編集内容は保存されています。");
    await refreshRef.current();
  }));
  const openPreview = (summary: MinutesVersion, restoring: boolean, button: HTMLElement) => {
    opener.current = button;
    return run(() => saved(async () => {
      const state = await invoke<MinutesVersions>("get_minutes_versions", { meetingId });
      assertMinutesVersion(state, summary, restoring);
      const [view, draft, present, presentDraft] = await Promise.all([
        invoke<MinutesView | null>("get_minutes_preview", { meetingId, minutesId: summary.id }),
        restoring ? invoke<MinutesDraft | null>("get_minutes_draft", { meetingId, minutesId: summary.id }) : Promise.resolve(null),
        state.currentId ? invoke<MinutesView | null>("get_minutes_preview", { meetingId, minutesId: state.currentId }) : Promise.resolve(null),
        state.currentId ? invoke<MinutesDraft | null>("get_minutes_draft", { meetingId, minutesId: state.currentId }) : Promise.resolve(null),
      ]);
      if (!view || view.contentHash !== summary.contentHash || presentDraft?.id !== state.draft?.id || presentDraft?.lockVersion !== state.draft?.lockVersion) throw new Error("比較内容の読込中に議事録・下書きが変更されています。再確認してください。");
      const checked = state.versions.find((v) => v.id === summary.id)!;
      setPreview({ summary: checked, original: state, view, draft, current: present, currentDraft: presentDraft, restoring });
    }));
  };
  const apply = (confirmationOpener: HTMLElement) => run(async () => {
    if (!preview) return;
    const state = await invoke<MinutesVersions>("get_minutes_versions", { meetingId });
    assertMinutesReview(state, preview.original, preview.summary, preview.restoring);
    const present = state.versions.find((v) => v.id === state.currentId);
    const previous = state.versions.find((v) => v.id === state.previousId);
    const message = preview.restoring ? `直前版 #${preview.summary.revisionNo} に戻しますか？ 現在版 #${present?.revisionNo} は新しい直前版になり、両方の保存済み編集を保持します。`
      : `候補 #${preview.summary.revisionNo} を採用しますか？ ${present ? `現在版 #${present.revisionNo} とその編集は新しい直前版へ移します。` : "初めての現在版になります。"}${previous ? `今の直前版 #${previous.revisionNo} はこの画面から復元できなくなります。保存済み編集と参照中のデータは削除しません。` : ""}`;
    await confirm({ opener: confirmationOpener, title: preview.restoring ? "議事録の直前版復元を確認" : "議事録候補の採用を確認", target: `会議「${meetingTitle}」 · 議事録 #${preview.summary.revisionNo}`, confirmLabel: preview.restoring ? "直前版へ戻す" : "候補を採用する", message, onError: cause => { if (mounted.current) setError(String(cause)); }, action: () => saved(async () => {
    if (preview.restoring) await invoke("restore_minutes_version", { meetingId, expectedMeetingLockVersion: state.meetingLockVersion, expectedPreviousId: preview.view.id, expectedContentHash: preview.view.contentHash, expectedDraft: state.draft });
    else await invoke("adopt_minutes_candidate", { request: { meetingId, revisionId: preview.view.id, expectedContentHash: preview.view.contentHash, expectedMeetingLockVersion: state.meetingLockVersion, expectedDraft: state.draft, confirmReplacingPrevious: !!state.previousId } });
    setPreview(null); loadedCurrent.current = "";
    await onHeadChanged(); await refreshRef.current();
    }) });
  });
  const engine = settings.payload?.[profile];
  const disabled = busy || locked;
  const pending = jobs.some((j) => ["queued", "running", "pausing"].includes(j.state));
  const previous = versions?.versions.find((v) => v.id === versions.previousId);
  const currentSummary = versions?.versions.find((v) => v.id === current?.id);
  return <section className="analysis-card minutes-card" aria-label="議事録生成と候補確認">
    <div className="card-head"><div className="heading-with-info"><h2>議事録</h2><InfoPopover label="議事録">修正済みの文字起こし・話者を生成開始時に固定します。生成結果の正しさは未確認です。内容と根拠を照合してから採用してください。</InfoPopover></div><button type="button" className="small-secondary" disabled={disabled} onClick={() => void run(() => refreshRef.current())}>議事録を再読込</button></div>
    <details className="analysis-settings"><summary>議事録エンジン設定（端末内に保存）</summary>
      <p>導入済みの llama.cpp <code>llama-completion</code> と日本語対応GGUFの絶対パスを指定してください。対話用 llama-cli は使用しません。標準・高精度のモデル品質と性能は未検証です。</p>
      <div className="info-field"><div className="heading-with-info"><label htmlFor={profileSelectId}>議事録プロファイル</label><InfoPopover label="議事録の標準／高精度">別々のモデルと数値設定を保存します。大きなモデルや入力上限はメモリ・処理時間に影響します。標準・高精度の品質と性能は未検証で、プロファイル名だけで精度を保証しません。</InfoPopover></div><select id={profileSelectId} disabled={disabled} value={profile} onChange={(e) => setProfile(e.target.value as typeof profile)}><option value="standard">標準</option><option value="highAccuracy">高精度</option></select></div>
      {engine ? <fieldset disabled={disabled}>
        <label>llama-completion の絶対パス<input value={engine.executablePath} maxLength={4096} onChange={(e) => update({ executablePath: e.target.value })} /></label>
        <label>GGUFモデルの絶対パス<input value={engine.modelPath} maxLength={4096} onChange={(e) => update({ modelPath: e.target.value })} /></label>
        <div className="engine-numbers">{([ ["threads", "議事録CPUスレッド数", 1, 64], ["contextTokens", "コンテキスト上限（tokens）", 2048, 32768], ["predictionTokens", "出力上限（tokens）", 64, 8192], ["gpuLayers", "GPUへ配置する層数", 0, 256], ["seed", "seed", 0, 4294967295], ["promptBudget", "抽出入力上限（bytes）", 2048, 60000] ] as const).map(([key, label, min, max]) => <label key={key}>{label}<input type="number" min={min} max={max} step={1} value={engine[key]} onChange={(e) => update({ [key]: Number(e.target.value) })} /></label>)}</div>
        <p>入力上限はコンテキスト − 出力上限 − 512 以下にしてください。bytesは正確なトークン数ではありません。GPU 0ならCPUで処理します。</p>
      </fieldset> : <p>議事録設定を読込中です。</p>}
      <div className="analysis-actions"><span role="status">{settings.error ? "議事録設定の保存に失敗" : settings.saving ? "議事録設定を保存中" : settings.dirty ? "議事録設定は未保存" : "議事録設定は保存済み"}</span><button type="button" className="small-secondary" disabled={disabled || settings.saving || !settings.dirty} onClick={() => void run(() => queue.retry())}>{settings.error ? "議事録設定保存を再試行" : "議事録設定を保存"}</button></div>
      {settings.error && <p className="action-error" role="alert">{settings.error} 入力は保持しています。</p>}
    </details>
    <div className="analysis-actions"><button type="button" disabled={disabled || recordingActive || pending || !source?.currentId || !engine || !minutesConfigured(engine)} onClick={() => void start()}>{current ? "議事録を再生成" : "議事録を生成"}</button><span>{profile === "highAccuracy" ? "高精度" : "標準"} · {recordingActive ? "録音を優先" : "端末内で処理"}</span></div>
    {!source?.currentId && <p>先に文字起こし候補を確認・採用してください。</p>}
    {jobs.filter((j) => j.kind === "minutes_generation").length > 0 && <ul className="analysis-jobs" aria-label="議事録生成ジョブ">{jobs.filter((j) => j.kind === "minutes_generation").map((j) => <li key={j.id}><div><strong>{jobLabel(j.state)}{j.attempt > 1 ? ` · 試行 ${j.attempt}` : ""}</strong><span>保存済み {j.completedUnits} / {j.totalUnits ?? "?"} 単位{j.pauseReason === "recording" ? " · 録音・取り込み優先で停止" : ""}</span><small>ジョブ {j.id.slice(0, 8)}{j.retryOfJobId ? ` · 再試行元 ${j.retryOfJobId.slice(0, 8)}` : ""}</small>{j.errorCode && <span className="action-error">{j.errorCode}</span>}</div><div className="analysis-actions">
      {["queued", "running"].includes(j.state) && <button type="button" className="small-secondary" disabled={disabled} onClick={() => void jobAction("pause_analysis_job", j.id)}>生成を一時停止</button>}
      {["paused", "interrupted"].includes(j.state) && <button type="button" className="small-secondary" disabled={disabled || recordingActive || pending} onClick={() => void jobAction("resume_analysis_job", j.id)}>生成を再開</button>}
      {["queued", "running", "paused", "interrupted"].includes(j.state) && <button type="button" className="small-secondary" disabled={disabled} onClick={() => void cancelJob(j)}>生成を取消</button>}
      {canRetryJob(j, jobs, "minutes_generation") && <button type="button" className="small-secondary" disabled={disabled || recordingActive || pending} onClick={() => void retryJob(j)}>議事録を同じ条件で再試行</button>}
      <JobModelRebinding meetingId={meetingId} job={j} jobs={jobs} locked={disabled} recordingActive={recordingActive} withSavedEdits={withSavedEdits} refresh={() => refreshRef.current()} />
    </div></li>)}</ul>}
    <div className="analysis-versions"><h3>議事録の確認待ち候補</h3>{versions?.versions.filter((v) => v.publicationState === "candidate").map((v) => <div className="version-row" key={v.id}><span>議事録候補 #{v.revisionNo} · {v.itemCount} 項目 {v.inputUpdated && <small>元データ更新あり</small>}</span><button type="button" className="small-secondary" disabled={disabled} onClick={(e) => void openPreview(v, false, e.currentTarget)}>議事録候補を確認</button></div>)}{versions && !versions.versions.some((v) => v.publicationState === "candidate") && <p>議事録の確認待ち候補はありません。</p>}{previous && <div className="version-row"><span>議事録直前版 #{previous.revisionNo}</span><button type="button" className="small-secondary" disabled={disabled} onClick={(e) => void openPreview(previous, true, e.currentTarget)}>議事録直前版を確認・復元</button></div>}</div>
    {current && <><div className="transcript-actions"><p>現在版 #{currentSummary?.revisionNo ?? "?"} {currentDraft && "· 保存済み編集を含む"}</p><button type="button" disabled={disabled} onClick={() => void writeMarkdown()}>{pendingExport.current ? "前回の書き出し結果を確認" : "Markdownを書き出す"}</button></div>{currentSummary?.inputUpdated && <p className="data-updated" role="status">元データ更新あり。引用は生成時の版を保持しています。必要に応じて再生成してください。</p>}<MinutesEditor key={current.id} view={current} meetingTitle={meetingTitle} initialDraft={currentDraft} locked={disabled} registerGuard={(guard) => { editorGuard.current = guard; }} /></>}
    {exportNoticeLevel(exportState) === "error" ? <IssueNotice scope={meetingId} type="minutes-export" label="議事録の書き出しを要確認" message={exportNotice} /> : <ToastNotice scope={meetingId} type="minutes-export" eventId={exportEvent} level={exportNoticeLevel(exportState)} message={exportNotice} onClose={() => setExportNotice("")} />}
    <details className="minutes-export-history"><summary>Markdown出力履歴（最新50件）</summary><p>書き出した時点の内容と結果です。外部ファイルの現在の存在・内容は自動追跡しません。</p>{exports.length === 0 ? <p>出力履歴はありません。</p> : <ul>{exports.map((record) => <li key={record.id}><strong>{exportLabel(record.state)}</strong> · <time>{new Date(record.exportedAt ?? record.createdAt).toLocaleString("ja-JP")}</time><p>{record.destinationPath}</p><small>元議事録: {record.minutesRevisionId ?? "整理済み"} · 内容SHA-256: {record.exportedContentHash} · ファイルSHA-256: {record.outputContentHash} · {record.outputSizeBytes} bytes</small>{record.errorMessage && <p className="action-error">{record.errorMessage}</p>}</li>)}</ul>}</details>
    {error && <p className="action-error" role="alert">{error}</p>}
    <CommonModal ref={dialog} open={!!preview} blocked={disabled} onDismiss={() => setPreview(null)} restoreReady={!disabled} restoreTarget={() => opener.current} className="analysis-dialog" aria-labelledby="minutes-preview-title">{preview && <>
      <div className="card-head"><h2 id="minutes-preview-title">{preview.restoring ? "議事録直前版の復元確認" : "議事録候補の採用確認"} #{preview.summary.revisionNo}</h2><button type="button" className="small-secondary" disabled={disabled} data-dialog-cancel onClick={() => setPreview(null)}>議事録プレビューを閉じる</button></div>
      <p>全項目の内容・根拠・担当者・期限を確認してください。SHA-256: <code>{preview.view.contentHash}</code></p>{preview.summary.inputUpdated && <p className="data-updated">元データ更新あり。この版の根拠は生成時の入力を保持しています。</p>}
      <details><summary>固定された議事録処理条件</summary><pre>{JSON.stringify({ profile: preview.summary.profile, inputAnalysisId: preview.view.inputAnalysisId, inputContentHash: preview.view.inputContentHash }, null, 2)}</pre></details>
      <div className="preview-columns">{preview.current ? <MinutesDocument key={`current-${preview.current.id}`} view={preview.current} locked={disabled} content={preview.currentDraft?.payload ?? preview.current.content} title="比較する現在の議事録" /> : <section><h3>現在の議事録</h3><p>まだありません。</p></section>}<MinutesDocument key={preview.view.id} view={preview.view} locked={disabled} content={preview.draft?.payload ?? preview.view.content} title={preview.restoring ? "復元する議事録（保存済み編集を含む）" : "採用する議事録候補"} /></div>
      {error && <p className="action-error" role="alert">{error}</p>}
      <div className="analysis-actions"><button type="button" disabled={disabled} onClick={event => void apply(event.currentTarget)}>{preview.restoring ? "この議事録直前版へ戻す" : "この議事録候補を採用"}</button><span>保存済み編集は版ごとに保持します。</span></div>
    </>}</CommonModal>
  </section>;
}
