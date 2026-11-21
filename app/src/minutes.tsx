import { invoke, isTauri } from "./ipc.ts";
import { subscribeInvalidations } from "./native-snapshots";
import { observeInvalidations } from "./snapshot-resource";
import { browserFocus, browserSchedule } from "./snapshot-browser";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { IconButton } from "./action-icon";
import { IssueNotice, ToastNotice } from "./toast";
import { useActionConfirmation } from "./confirmation-service";
import { registerPendingEdits, type VersionedDraft } from "./draft-save";
import { SavedEditsCancelled, withConfirmedEdits } from "./confirmed-action";
import { jobLabel, type AnalysisJob, type MinutesSource } from "./analysis-state";
import { minutesConfigured, type MinutesPreferences } from "./minutes-state";
import { MinutesMarkdownEditor } from "./minutes-markdown-editor";
import { MinutesMarkdownPreview } from "./minutes-markdown-preview";
import { MarkdownSaveController, markdownStamp, type MarkdownDocument } from "./minutes-markdown-state";
import { confirmMarkdownLeave } from "./minutes-markdown-leave";
import { MarkdownExportController, type MarkdownExportRecord, type MarkdownExportStatus } from "./minutes-markdown-export";
import "./minutes-markdown-editor.css";
import { validateAudioSummary, type MeetingAudioSummary } from "./minutes-audio-state";
import { useMeetingAudio } from "./meeting-audio";
import { topOpenModal } from "./modal-state";
import type { ConfirmationOptions } from "./confirmation-state";
import { minutesAdmissions } from "./minutes-generation-admission";
import { readMeetingBodyForReview } from "./meeting-body-review";
import type { MeetingBodyReview } from "./generated/contracts";

type Job = AnalysisJob & { operationId: string };

/** A single human-owned Markdown body, not a candidate or revision editor. */
export function MinutesPanel({ meetingId, meetingTitle, sourceEpoch, recordingActive, locked, navigationLocked = false, withSavedEdits, visible = true }: {
  meetingId: string; meetingTitle: string; sourceEpoch: number; recordingActive: boolean; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void | "cancelled">;
  onHeadChanged: () => Promise<void>; visible?: boolean; navigationLocked?: boolean;
}) {
  const documentWritable = useRef(false);
  const [controller] = useState(() => new MarkdownSaveController(meetingId, async request => {
    if (!documentWritable.current) throw Error("議事録の保存状態を確認できません。入力を保持しています。");
    return invoke<MarkdownDocument>("save_minutes_markdown", { request });
  }));
  const [documentState, setDocumentState] = useState(() => controller.snapshot());
  const [exporter] = useState(() => new MarkdownExportController(meetingId,
    request => invoke<MarkdownExportRecord|null>("export_plain_minutes_markdown", { request }),
    request => invoke<MarkdownExportStatus>("get_plain_minutes_export_status", { request })));
  const [exportState, setExportState] = useState(() => exporter.snapshot());
  const [exportedEvent, setExportedEvent] = useState("");
  const exportOpener = useRef<HTMLButtonElement|null>(null), returnExportFocus = useRef(false);
  const [source, setSource] = useState<MinutesSource | null>(null);
  const [storedAudioSummary, setAudioSummary] = useState<MeetingAudioSummary | null>(null);
  const audio = useMeetingAudio(meetingId);
  const canInspectSource = audio?.canInspectSource ?? true;
  const sourceAvailable = useRef(canInspectSource); sourceAvailable.current = canInspectSource;
  const audioSummary = canInspectSource ? storedAudioSummary : null;
  const [settings, setSettings] = useState<MinutesPreferences | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [loaded, setLoaded] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [bodyError, setBodyError] = useState("");
  const [review, setReview] = useState<MeetingBodyReview | null>(null);
  const [sourceError, setSourceError] = useState(""), [settingsError, setSettingsError] = useState(""), [jobsError, setJobsError] = useState("");
  const [externalChange, setExternalChange] = useState(false);
  const [actionsHost, setActionsHost] = useState<HTMLDivElement | null>(null);
  const alive = useRef(true), running = useRef(false);
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const admissions = useSyncExternalStore(minutesAdmissions.subscribe, minutesAdmissions.getSnapshot);
  const preparing = admissions.some(item => item.meetingId === meetingId && item.pending);
  // Navigation/quit sets a main-window lock while awaiting this decision.
  // That lock must not disable the confirmation which resolves it.
  const confirm = useActionConfirmation(false);
  const editorInput = useRef<HTMLTextAreaElement|null>(null);
  const confirmLeave = (options: ConfirmationOptions) => {
    // Native quit disables the editor before its async status reads finish.
    // Capture the persistent input explicitly, not the resulting body focus.
    // A nested review keeps its own modal entry point instead.
    const input = editorInput.current;
    return confirm({ ...options, opener: !topOpenModal() && input?.isConnected
      && !!input.getClientRects().length ? input : undefined });
  };
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
      // Publishing transcription does not leave or replace the human's
      // Markdown. Never save/prompt/discard it as a side effect of ASR.
      if (reason === "transcription") return;
      // Both tab panels stay mounted. This is not leaving the meeting and
      // must not prompt or write the human's unfinished Markdown input.
      if (reason === "tab" && !controller.isComposing()) return;
      if (live.current.loaded) await confirmMarkdownLeave(controller, live.current.meetingTitle, read, confirmLeave);
    }, reason => {
      if (!live.current.loaded || reason === "transcription") return false;
      if (controller.isComposing()) return true;
      if (reason === "tab") return false;
      const current = controller.snapshot();
      return current.dirty || current.saving;
    }, () => {
      if (controller.isComposing()) throw Error("議事録の日本語入力を確定してから退避してください。");
      const current = controller.snapshot();
      return { kind: "minutes", meetingId, input: { markdown: current.text, base: markdownStamp(current.document) } };
    });
    return () => { alive.current = false; unsubscribe(); unregister(); };
  }, [controller]);
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    const observe = (refresh: () => Promise<void>, onError: (cause: unknown) => void) => observeInvalidations({
      read: refresh,
      subscribe: notify => subscribeInvalidations(["analysis-job-changed", "analysis-version-changed", "model-profile-changed"], notify),
      error: cause => { if (!disposed) onError(cause); },
      intervalMs: 3000,
      schedule: browserSchedule,
      focus: browserFocus,
    });
    const document = observe(async () => {
      try {
        const saved = await read();
        if (disposed) return;
        const matched = controller.observe(saved);
        setExternalChange(!matched); setBodyError(""); setReview(null); setLoaded(true);
        documentWritable.current = true;
      } catch (cause) {
        if (disposed) return;
        documentWritable.current = false;
        setBodyError(String(cause)); setLoaded(true);
        const fallback = await readMeetingBodyForReview(meetingId, "minutes").catch(() => null);
        if (!disposed) setReview(fallback);
      }
    }, () => { /* An event subscription failure is recovered by polling. */ });
    const transcript = observe(async () => {
      const value = await invoke<MinutesSource>("get_minutes_source", { meetingId });
      if (!disposed) { setSource(value); setSourceError(""); }
    }, cause => { setSource(null); setSourceError(String(cause)); });
    const preferences = observe(async () => {
      const value = await invoke<VersionedDraft<MinutesPreferences>>("get_minutes_preferences");
      if (!disposed) { setSettings(value.payload); setSettingsError(""); }
    }, cause => { setSettings(null); setSettingsError(String(cause)); });
    const jobList = observe(async () => {
      const rows = await invoke<Job[]>("get_analysis_jobs", { meetingId });
      if (!Array.isArray(rows)) throw Error("議事録生成のジョブを確認できません。");
      if (!disposed) { setJobs(rows); setJobsError(""); minutesAdmissions.observe(meetingId, rows.map(job => job.operationId)); }
    }, cause => { setJobsError(String(cause)); });
    const audio = observe(async () => {
      const value = sourceAvailable.current ? validateAudioSummary(await invoke<MeetingAudioSummary>("get_meeting_audio_summary", { meetingId })) : null;
      if (!disposed) setAudioSummary(value);
    }, () => setAudioSummary(null));
    const resources = [document, transcript, preferences, jobList, audio];
    refreshRef.current = async () => { for (const resource of resources) void resource.refresh(); };
    return () => { disposed = true; resources.forEach(resource => resource.stop()); };
  }, [meetingId, controller]);
  useEffect(() => { void refreshRef.current(); }, [sourceEpoch]);
  const run = async (action: () => Promise<void>) => {
    if (running.current || locked || !isTauri()) return;
    running.current = true; setBusy(true); setError("");
    try { await action(); }
    catch (cause) { if (alive.current && !(cause instanceof SavedEditsCancelled)) setError(String(cause)); }
    finally { running.current = false; if (alive.current) setBusy(false); }
  };
  const start = () => run(async () => {
    if (!documentWritable.current) throw Error("議事録の保存状態を確認できません。再読込してください。");
    let submit: (() => Promise<void>) | undefined;
    await withConfirmedEdits(withSavedEdits, async () => {
    const saved = await read();
    if (!controller.observe(saved)) throw Error("保存済みの議事録が変更されています。入力を保持し、再読込から確認してください。");
    const prepare = async () => {
      const preferences = await invoke<VersionedDraft<MinutesPreferences>>("get_minutes_preferences");
      // The global model picker writes the standard slot. Read it again at
      // admission (after replacement confirmation), never a meeting override.
      const engine = preferences.payload.standard;
      if (!engine || !minutesConfigured(engine)) throw Error("設定のローカルAIで議事録用モデルを導入してください。");
      const analysis = await invoke<MinutesSource>("get_minutes_source", { meetingId });
      if (!analysis.currentId || !analysis.contentHash) throw Error("先に文字起こしを完了してください。");
      const base = { input: { meetingId, expectedAnalysisId: analysis.currentId, expectedContentHash: analysis.contentHash,
        expectedMeetingLockVersion: analysis.meetingLockVersion, expectedDraft: analysis.draft },
        ...engine, profile: "standard" };
      const expected = markdownStamp(saved), key = JSON.stringify({ base, expected });
      // Freeze the confirmed inputs while the edit barrier is held. The IPC
      // starts only after that barrier and the confirmation dialog are released.
      submit = () => minutesAdmissions.start(meetingId, meetingTitle, key, async operationId => {
        await invoke("start_markdown_minutes_generation", { request: { ...base, input: { ...base.input, operationId } }, expected });
      });
    };
    if (saved) await confirm({ title: "議事録を再生成", target: `会議「${meetingTitle}」`,
      message: "生成に成功すると保存済みの議事録を置き換えます。手動編集も置き換わり、旧版は保存しません。失敗した場合は現在の本文を保持します。",
      confirmLabel: "議事録を再生成", validate: () => alive.current && !controller.snapshot().dirty
        && controller.snapshot().document?.operationId === saved.operationId, action: prepare,
      onError: cause => {
        if (!alive.current) return false;
        setError(String(cause));
        return true;
      } });
    else await prepare();
    });
    if (submit) {
      // The main-window admission owner also reports failures after this
      // meeting view unmounts; do not emit a second local error toast.
      try { await submit(); } catch { return; }
      if (alive.current) await refreshRef.current();
    }
  });
  const exportMarkdown = (opener:HTMLButtonElement) => run(async () => {
    exportOpener.current = opener;
    // WebKit mouse activation otherwise leaves focus on the containing main.
    // Bind focus before disabling this icon so cancellation can restore it.
    opener.focus({ preventScroll: true });
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
  const generationJobs = jobs.filter(j => j.kind === "minutes_generation");
  const pending = generationJobs.some(j => ["queued", "running", "pausing", "paused", "interrupted"].includes(j.state));
  const disabled = locked || busy || !loaded || !isTauri();
  const engine = settings?.standard;
  const cannotGenerate = disabled || !!bodyError || !!jobsError || externalChange || recordingActive || pending || preparing || !source?.currentId || !engine || !minutesConfigured(engine);
  return <section className="analysis-card minutes-card plain-minutes-card" aria-label="議事録">
    <div className="card-head"><h2>議事録</h2><div className="plain-minutes-primary-actions">
      <div ref={setActionsHost} className="plain-minutes-editor-actions-host" />
      <IconButton label={exportState.needsCheck ? "書き出し結果を確認" : exportState.retry ? "議事録の書き出しを再試行" : "議事録をMarkdownで保存"} icon="download"
        disabled={disabled || !!bodyError || exportState.busy || (!documentState.document && !exportState.needsCheck && !exportState.retry)} onClick={event => void exportMarkdown(event.currentTarget)} />
      <button type="button" disabled={cannotGenerate} onClick={() => void start()}>議事録を生成</button>
      {generationJobs.filter(j => ["queued", "running", "pausing", "paused", "interrupted"].includes(j.state)).map(j => <div key={j.id} className="analysis-actions">
        {["queued", "running"].includes(j.state) && <IconButton label="生成を一時停止" icon="pause" disabled={disabled} onClick={() => void jobAction("pause_analysis_job", j.id)} />}
        {["paused", "interrupted"].includes(j.state) && <button type="button" disabled={disabled || recordingActive} onClick={() => void jobAction("resume_analysis_job", j.id)}>生成を再開</button>}
        <IconButton label="生成を取り消す" icon="close" disabled={disabled} onClick={() => void cancelJob(j)} />
      </div>)}
    </div></div>
    {pending && <p className="minutes-generation-status" role="status">議事録生成：{generationJobs.filter(j => ["queued", "running", "pausing", "paused", "interrupted"].includes(j.state)).map(j => `${jobLabel(j.state)} ${j.completedUnits} / ${j.totalUnits ?? "?"}`).join("、")}</p>}
    {externalChange && <p className="plain-minutes-save-error" role="status">保存済みの本文が更新されています。編集中の入力は保持しています。再読込から保存済みの本文を確認できます。</p>}
    {!loaded && <p className="minutes-empty-state">議事録を読み込んでいます。</p>}
    {bodyError && <p className="plain-minutes-save-error" role="status">保存済みの議事録を確認できませんでした。{review && "読み取れた本文を閲覧専用で表示しています。"} {documentState.dirty && "編集中の内容は残っています。"}
      <IconButton label="議事録を再読込" icon="refresh" disabled={navigationLocked} onClick={() => void refreshRef.current()} /></p>}
    {review && <section aria-label="確認用の議事録本文"><MinutesMarkdownPreview text={review.text} durationUs={null} /></section>}
    {(sourceError || settingsError || jobsError) && <p className="meeting-document-notice" role="status">{[sourceError && "文字起こし", settingsError && "生成設定", jobsError && "生成の進み具合"].filter(Boolean).join("、")}を確認できませんでした。現在は議事録を生成できません。</p>}
    {loaded && (!bodyError || documentState.dirty) && <MinutesMarkdownEditor meetingId={meetingId} controller={controller} locked={locked || busy || !!bodyError} navigationLocked={navigationLocked || busy} visible={visible} actionsHost={actionsHost} inputRef={editorInput}
      durationUs={audioSummary?.modes.length ? audioSummary.durationUs : null}
      onPlay={audio?.canPlay && !audio.locked && audioSummary ? interval => { void audio.playReference({ kind:"interval", ...interval, expectedDurationUs:audioSummary.durationUs }); } : undefined} />}
    <ToastNotice scope={meetingId} type="minutes-integrity" level="warning" meetingId={meetingId} historyOnly message={bodyError ? "議事録の保存状態に確認が必要です。" : ""} />
    <IssueNotice scope={meetingId} type="minutes-operation" label="議事録操作" message={error} />
    <IssueNotice scope={meetingId} type="plain-minutes-export-error" label="議事録の書き出し" message={exportState.error} />
    <ToastNotice scope={meetingId} type="plain-minutes-export" eventId={exportedEvent} message={exportedEvent ? "議事録をMarkdownで保存しました" : ""} />
  </section>;
}
