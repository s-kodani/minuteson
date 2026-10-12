import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { IconButton } from "./action-icon";
import { OptionsModal } from "./options-modal";
import { IssueNotice, ToastNotice } from "./toast";
import { useActionConfirmation } from "./confirmation-service";
import { registerPendingEdits, type VersionedDraft } from "./draft-save";
import { withConfirmedEdits } from "./confirmed-action";
import { canRetryJob, jobLabel, type AnalysisJob, type VersionState } from "./analysis-state";
import { minutesConfigured, type MinutesPreferences } from "./minutes-state";
import { MinutesMarkdownEditor } from "./minutes-markdown-editor";
import { MarkdownSaveController, markdownStamp, type MarkdownDocument } from "./minutes-markdown-state";
import { confirmMarkdownLeave } from "./minutes-markdown-leave";
import { MarkdownExportController, type MarkdownExportRecord, type MarkdownExportStatus } from "./minutes-markdown-export";
import "./minutes-markdown-editor.css";

type Job = AnalysisJob & { operationId: string };

/** A single human-owned Markdown body, not a candidate or revision editor. */
export function MinutesPanel({ meetingId, meetingTitle, sourceEpoch, recordingActive, locked, withSavedEdits, visible = true }: {
  meetingId: string; meetingTitle: string; sourceEpoch: number; recordingActive: boolean; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  onHeadChanged: () => Promise<void>; visible?: boolean;
}) {
  const [controller] = useState(() => new MarkdownSaveController(meetingId,
    request => invoke<MarkdownDocument>("save_minutes_markdown", { request })));
  const [documentState, setDocumentState] = useState(() => controller.snapshot());
  const [exporter] = useState(() => new MarkdownExportController(meetingId,
    request => invoke<MarkdownExportRecord|null>("export_plain_minutes_markdown", { request }),
    request => invoke<MarkdownExportStatus>("get_plain_minutes_export_status", { request })));
  const [exportState, setExportState] = useState(() => exporter.snapshot());
  const [exportedEvent, setExportedEvent] = useState("");
  const exportOpener = useRef<HTMLButtonElement|null>(null), returnExportFocus = useRef(false);
  const [source, setSource] = useState<VersionState | null>(null);
  const [settings, setSettings] = useState<MinutesPreferences | null>(null);
  const [profile, setProfile] = useState<"standard" | "highAccuracy">("standard");
  const [jobs, setJobs] = useState<Job[]>([]);
  const [loaded, setLoaded] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [externalChange, setExternalChange] = useState(false);
  const [actionsHost, setActionsHost] = useState<HTMLDivElement | null>(null);
  const alive = useRef(true), running = useRef(false);
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const startOperations = useRef(new Map<string, string>()), retryOperations = useRef(new Map<string, string>());
  // Navigation/quit sets a main-window lock while awaiting this decision.
  // That lock must not disable the confirmation which resolves it.
  const confirm = useActionConfirmation(false);
  const live = useRef({ meetingTitle, loaded }); live.current = { meetingTitle, loaded };
  const read = () => invoke<MarkdownDocument | null>("get_minutes_markdown", { meetingId });
  useEffect(() => exporter.subscribe(() => setExportState(exporter.snapshot())), [exporter]);
  useEffect(() => {
    if (!returnExportFocus.current || locked || busy || exportState.busy || !visible) return;
    returnExportFocus.current = false;
    const target = exportOpener.current, active = document.activeElement;
    if (target?.isConnected && !target.disabled && !target.closest("[hidden],[inert]")
      && !document.querySelector("dialog[open]") && (active === document.body || active === target || !active?.isConnected)) target.focus({ preventScroll: true });
  }, [locked, busy, exportState.busy, visible]);
  useEffect(() => {
    alive.current = true;
    const unsubscribe = controller.subscribe(() => setDocumentState(controller.snapshot()));
    const unregister = registerPendingEdits(async reason => {
      // Both tab panels stay mounted. This is not leaving the meeting and
      // must not prompt or write the human's unfinished Markdown input.
      if (reason === "tab" && !controller.isComposing()) return;
      if (live.current.loaded) await confirmMarkdownLeave(controller, live.current.meetingTitle, read, confirm);
    });
    return () => { alive.current = false; unsubscribe(); unregister(); };
  }, [controller]);
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false, inFlight: Promise<void> | null = null, again = false;
    const stops: (() => void)[] = [];
    const refresh = (): Promise<void> => {
      if (disposed) return Promise.resolve();
      if (inFlight) { again = true; return inFlight; }
      inFlight = (async () => {
        do {
          again = false;
          try {
            const [saved, analysis, preferences, allJobs] = await Promise.all([
              read(), invoke<VersionState>("get_analysis_versions", { meetingId }),
              invoke<VersionedDraft<MinutesPreferences>>("get_minutes_preferences"),
              invoke<Job[]>("get_analysis_jobs", { meetingId }),
            ]);
            if (disposed) return;
            setExternalChange(!controller.observe(saved));
            setSource(analysis); setSettings(preferences.payload); setJobs(allJobs); setLoaded(true);
            for (const [key, operation] of startOperations.current) if (allJobs.some(j => j.operationId === operation)) startOperations.current.delete(key);
            for (const [key, operation] of retryOperations.current) if (allJobs.some(j => j.operationId === operation)) retryOperations.current.delete(key);
          } catch (cause) { if (!disposed) setError(String(cause)); }
        } while (again && !disposed);
      })().finally(() => { inFlight = null; });
      return inFlight;
    };
    refreshRef.current = refresh;
    for (const event of ["analysis-job-changed", "analysis-version-changed", "minutes-version-changed", "model-profile-changed"]) {
      void listen(event, () => { void refresh(); }).then(stop => {
        if (disposed) stop(); else { stops.push(stop); void refresh(); }
      }).catch(cause => { if (!disposed) setError(String(cause)); });
    }
    void refresh();
    const timer = window.setInterval(() => void refresh(), 3000);
    return () => { disposed = true; stops.forEach(stop => stop()); window.clearInterval(timer); };
  }, [meetingId, controller]);
  useEffect(() => { void refreshRef.current(); }, [sourceEpoch]);
  const run = async (action: () => Promise<void>) => {
    if (running.current || locked || !isTauri()) return;
    running.current = true; setBusy(true); setError("");
    try { await action(); }
    catch (cause) { if (alive.current) setError(String(cause)); }
    finally { running.current = false; if (alive.current) setBusy(false); }
  };
  const start = () => run(() => withConfirmedEdits(withSavedEdits, async () => {
    const saved = await read();
    if (!controller.observe(saved)) throw Error("保存済みの議事録が変更されています。入力を保持し、再読込から確認してください。");
    const enqueue = async () => {
      const preferences = await invoke<VersionedDraft<MinutesPreferences>>("get_minutes_preferences");
      const engine = preferences.payload[profile];
      if (!engine || !minutesConfigured(engine)) throw Error("設定のローカルAIで議事録用モデルを導入してください。");
      const analysis = await invoke<VersionState>("get_analysis_versions", { meetingId });
      const version = analysis.versions.find(v => v.id === analysis.currentId);
      if (!version) throw Error("先に文字起こしを確認・採用してください。");
      const base = { input: { meetingId, expectedAnalysisId: version.id, expectedContentHash: version.contentHash,
        expectedMeetingLockVersion: analysis.meetingLockVersion, expectedDraft: analysis.draft },
        ...engine, profile: profile === "highAccuracy" ? "high_accuracy" : "standard" };
      const expected = markdownStamp(saved), key = JSON.stringify({ base, expected });
      const operationId = startOperations.current.get(key) ?? crypto.randomUUID();
      startOperations.current.set(key, operationId);
      await invoke("start_markdown_minutes_generation", { request: { ...base, input: { ...base.input, operationId } }, expected });
      startOperations.current.delete(key); await refreshRef.current();
    };
    if (saved) await confirm({ title: "議事録を再生成", target: `会議「${meetingTitle}」`,
      message: "生成に成功すると保存済みの議事録を置き換えます。手動編集も置き換わり、旧版は保存しません。失敗した場合は現在の本文を保持します。",
      confirmLabel: "議事録を再生成", validate: () => alive.current && !controller.snapshot().dirty
        && controller.snapshot().document?.operationId === saved.operationId, action: enqueue,
      onError: cause => { if (alive.current) setError(String(cause)); } });
    else await enqueue();
  }));
  const exportMarkdown = (opener:HTMLButtonElement) => run(async () => {
    exportOpener.current = opener;
    // Recovery is read-only and never prompts to save new input or reopens a
    // native dialog. It belongs to the original frozen export operation.
    if (exporter.snapshot().needsCheck) {
      const result = await exporter.check();
      if (result?.state === "succeeded") setExportedEvent(result.id);
      returnExportFocus.current = !exporter.snapshot().needsCheck;
      return;
    }
    await withConfirmedEdits(withSavedEdits, async () => {
      const saved = await read();
      if (!saved || !controller.observe(saved)) throw Error("保存済みの議事録を確認してください。編集中の入力は保持しています。");
      const result = await exporter.begin(saved);
      if (result?.state === "succeeded") setExportedEvent(result.id);
      returnExportFocus.current = true;
    });
  });
  const jobAction = (command: string, jobId: string) => run(async () => {
    await invoke(command, { jobId }); await refreshRef.current();
  });
  const cancelJob = (job: Job) => run(async () => {
    await confirm({ title: "議事録生成を取り消す", target: `会議「${meetingTitle}」`,
      message: "生成を取り消します。保存済みの議事録と編集中の入力は保持します。", confirmLabel: "生成を取り消す",
      action: async () => { await invoke("cancel_analysis_job", { jobId: job.id }); await refreshRef.current(); } });
  });
  const retryJob = (job: Job) => run(async () => {
    await confirm({ title: "議事録生成を再試行", target: `会議「${meetingTitle}」`,
      message: "開始時の文字起こし・モデル・保存条件を使い、保存済みの進捗から再試行します。後から保存した人間の本文は置き換えません。",
      confirmLabel: "同じ条件で再試行", action: async () => {
        const operationId = retryOperations.current.get(job.id) ?? crypto.randomUUID();
        retryOperations.current.set(job.id, operationId);
        await invoke("retry_minutes_generation", { request: { meetingId, jobId: job.id, operationId, expectedSpecHash: job.specHash } });
        retryOperations.current.delete(job.id); await refreshRef.current();
      } });
  });
  const generationJobs = jobs.filter(j => j.kind === "minutes_generation");
  const pending = generationJobs.some(j => ["queued", "running", "pausing", "paused", "interrupted"].includes(j.state));
  const disabled = locked || busy || !loaded || !isTauri();
  const engine = settings?.[profile];
  const cannotGenerate = disabled || recordingActive || pending || !source?.currentId || !engine || !minutesConfigured(engine);
  return <section className="analysis-card minutes-card plain-minutes-card" aria-label="議事録">
    <div className="card-head"><h2>議事録</h2><div className="plain-minutes-primary-actions">
      <div ref={setActionsHost} className="plain-minutes-editor-actions-host" />
      <IconButton label={exportState.needsCheck ? "書き出し結果を確認" : exportState.retry ? "議事録の書き出しを再試行" : "議事録をMarkdownで保存"} icon="download"
        disabled={disabled || exportState.busy || (!documentState.document && !exportState.needsCheck && !exportState.retry)} onClick={event => void exportMarkdown(event.currentTarget)} />
      {!documentState.document && <button type="button" disabled={cannotGenerate} onClick={() => void start()}>議事録を生成</button>}
      <OptionsModal label="議事録の操作" icon="more" hidden={!visible} className="plain-minutes-options">
        <div className="plain-minutes-options-body">
          <div className="plain-minutes-option-actions"><IconButton label="議事録を再読込" icon="refresh" disabled={disabled} onClick={() => void run(async () => {
            await confirmMarkdownLeave(controller, meetingTitle, read, confirm); await refreshRef.current();
          })} />{documentState.document && <button type="button" disabled={cannotGenerate} onClick={() => void start()}>議事録を再生成</button>}</div>
          <label>生成に使うモデル<select disabled={disabled || pending} value={profile} onChange={event => setProfile(event.target.value as typeof profile)}><option value="standard">標準</option><option value="highAccuracy">高精度</option></select></label>
          {(!engine || !minutesConfigured(engine)) && <p>設定のローカルAIから議事録用モデルを導入してください。</p>}
          {generationJobs.length > 0 && <ul className="analysis-jobs" aria-label="議事録生成の状況">{generationJobs.map(j => <li key={j.id}>
            <div><strong>{jobLabel(j.state)}</strong><span>{j.completedUnits} / {j.totalUnits ?? "?"}</span>{j.errorCode && <span className="action-error">{j.errorCode}</span>}</div>
            <div className="analysis-actions">
              {["queued", "running"].includes(j.state) && <IconButton label="生成を一時停止" icon="pause" disabled={disabled} onClick={() => void jobAction("pause_analysis_job", j.id)} />}
              {["paused", "interrupted"].includes(j.state) && <button type="button" className="small-secondary" disabled={disabled || recordingActive} onClick={() => void jobAction("resume_analysis_job", j.id)}>生成を再開</button>}
              {["queued", "running", "paused", "interrupted"].includes(j.state) && <button type="button" className="small-secondary" disabled={disabled} onClick={() => void cancelJob(j)}>生成を取消</button>}
              {canRetryJob(j, jobs, "minutes_generation") && <button type="button" className="small-secondary" disabled={disabled || recordingActive || pending} onClick={() => void retryJob(j)}>再試行</button>}
            </div>
          </li>)}</ul>}
        </div>
      </OptionsModal>
    </div></div>
    {pending && <p className="minutes-generation-status" role="status">議事録生成：{generationJobs.filter(j => ["queued", "running", "pausing", "paused", "interrupted"].includes(j.state)).map(j => `${jobLabel(j.state)} ${j.completedUnits} / ${j.totalUnits ?? "?"}`).join("、")}</p>}
    {externalChange && <p className="plain-minutes-save-error" role="status">保存済みの本文が更新されています。編集中の入力は保持しています。再読込から保存済みの本文を確認できます。</p>}
    {!loaded && <p className="minutes-empty-state">議事録を読み込んでいます。</p>}
    {loaded && <MinutesMarkdownEditor meetingId={meetingId} controller={controller} locked={locked || busy} visible={visible} actionsHost={actionsHost}
      durationUs={null} />}
    <IssueNotice scope={meetingId} type="minutes-operation" label="議事録操作" message={error} toastMessage="議事録操作を完了できません。入力を保持しています" />
    <IssueNotice scope={meetingId} type="plain-minutes-export-error" label="議事録の書き出し" message={exportState.error} toastMessage="議事録の書き出し結果を確認してください" />
    <ToastNotice scope={meetingId} type="plain-minutes-export" eventId={exportedEvent} message={exportedEvent ? "議事録をMarkdownで保存しました" : ""} />
  </section>;
}
