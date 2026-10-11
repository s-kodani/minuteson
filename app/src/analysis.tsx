import { invoke, isTauri } from "@tauri-apps/api/core";
import { IconButton } from "./action-icon";
import { InfoPopover } from "./info-popover";
import { IssueNotice, IssueStatus } from "./toast";
import { useVersionPublication } from "./version-publication";
import { useActionConfirmation } from "./confirmation-service";
import { CommonModal, type ModalReplacement } from "./common-modal";
import { OptionsModal } from "./options-modal";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { DraftSaveQueue, registerPendingEdits, type VersionedDraft } from "./draft-save";
import { assertReviewedVersion, canRetryJob, engineConfigured, jobLabel, type AnalysisJob, type AnalysisPreferences, type EnginePreferences, type VersionState, type VersionSummary } from "./analysis-state";
import { timeLabel, type Analysis, type Draft } from "./transcript";
import { VocabularyEditor } from "./vocabulary";
import type { VocabularyStamp } from "./vocabulary-state";
import { DiarizationPanel } from "./diarization-panel";
import { JobModelRebinding } from "./model-rebinding";
import { AnalysisFlowStatus } from "./analysis-flow";
import type { AnalysisFlow } from "./analysis-flow-state";
import { SpeakerReviewBadge,SpeakerReviewDetails,useSpeakerReviews } from "./speaker-review";
import { useTranscriptAudio } from "./transcript-audio";
import { projectTranscript, type Utterance } from "./transcript-state";

type Preferences = VersionedDraft<AnalysisPreferences>;
type Preview = { summary: VersionSummary; analysis: Analysis; draft: Draft | null; current: Analysis | null; currentDraft: Draft | null; restoring: boolean };

function PreviewText({ analysis, draft, title,locked }: { analysis: Analysis; draft: Draft | null; title: string;locked:boolean }) {
  const [page, setPage] = useState(0);
  const payload=draft?.baseAnalysisId===analysis.id?draft.payload:null;
  const view=projectTranscript(analysis,payload);
  const shown=view.utterances.slice(page*50,(page+1)*50);
  const review=useSpeakerReviews(analysis,[...new Set(shown.map(u=>u.sourceUtteranceId??u.id))]);
  const audio=useTranscriptAudio(analysis.meetingId,locked);
  const labels=new Map(view.speakers.map(s=>[s.id,(s.displayName||s.label)+(payload?.speaker_merges?.some(m=>m.source_id===s.id)?"（統合前の推定ラベル）":"")]));
  const play=(item:Utterance,at?:number)=>audio.start(analysis.id,item.id,item.sourceUtteranceId&&draft?{
    meetingId:analysis.meetingId,analysisId:analysis.id,expectedContentHash:analysis.contentHash,expectedDraft:{id:draft.id,lockVersion:draft.lockVersion},utteranceId:item.id,expectedStartUs:item.startUs,expectedEndUs:item.endUs,
  }:undefined,at);
  return <section className="preview-text" aria-label={title}>
    <h3>{title} {draft && <small>保存済み編集を含む</small>}</h3>
    {analysis.utterances.length === 0 && <p>発言はありません。</p>}
    {audio.controls}
    {review.error&&<p className="action-error" role="alert">推定区間を照合できません: {review.error} <IconButton label="推定表示を再読込" icon="refresh" type="button" disabled={locked} onClick={review.retry} /></p>}
    {shown.map((item) => {
      const source=analysis.utterances.find(u=>u.id===(item.sourceUtteranceId??item.id))!;
      const speakers = item.speakerIds.map((id) => {
        const speaker = view.speakers.find((value) => value.id === id);
        return speaker?.displayName || speaker?.label || "話者不明";
      });
      return <article key={item.id}>
        <div className="preview-utterance-meta">
          <small>{timeLabel(item.startUs)}–{timeLabel(item.endUs)} · {speakers.join(" / ") || "話者不明"}</small>
          <button type="button" className="small-secondary utterance-play-icon" aria-label={`${timeLabel(item.startUs)}の発言を再生`} title="この発言を再生" disabled={locked||audio.preparing} onClick={()=>void play(item)}>
            <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" focusable="false"><path d="M8 5v14l11-7z" fill="currentColor"/></svg>
          </button>
          {item.sourceUtteranceId?<small>手動分割 · 推定区間は分割元全体</small>:<SpeakerReviewBadge review={review.reviews.get(item.id)}/>}
        </div>
        <p>{item.effectiveText}</p>
        {item.recognitionIsSource&&<small>分割元の認識原文（全文・単語位置未推定）: {item.recognizedText}</small>}
        <SpeakerReviewDetails analysis={analysis} utteranceId={source.id} startUs={source.startUs} labels={labels} locked={locked||audio.preparing} onPlay={at=>audio.start(analysis.id,source.id,undefined,at)}/>
      </article>;
    })}
    {view.utterances.length > 50 && <div className="transcript-pages"><IconButton label="前へ" icon="previous" type="button" disabled={page === 0} onClick={() => setPage(page - 1)} /><span>{page + 1} / {Math.ceil(view.utterances.length / 50)}</span><IconButton label="次へ" icon="next" type="button" disabled={(page + 1) * 50 >= view.utterances.length} onClick={() => setPage(page + 1)} /></div>}
  </section>;
}

export function AnalysisPanel({ meetingId, meetingTitle, recorded, recordingActive, locked, withSavedEdits, onHeadChanged, diarizationHost = null, diarizationVisible = true, renderTools }: {
  meetingId: string; meetingTitle: string; recorded: boolean; recordingActive: boolean; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  onHeadChanged: () => Promise<void>;
  diarizationHost?: HTMLElement | null;
  diarizationVisible?: boolean;
  // The workspace owns the native settings boundary. Review replaces its
  // visible view, while this controller and the original fields stay mounted.
  renderTools?: (tools: ReactNode, review: ModalReplacement | null, persistentViews: ReactNode) => ReactNode;
}) {
  const [diarizationMount] = useState(() => document.createElement("div"));
  const localDiarization = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => { (diarizationHost ?? localDiarization.current)?.append(diarizationMount); }, [diarizationHost,diarizationMount]);
  useLayoutEffect(() => () => diarizationMount.remove(), [diarizationMount]);
  const profileSelectId = useId();
  const engineTitleId = useId();
  const engineTrigger = useRef<HTMLButtonElement>(null);
  const [engineOpen, setEngineOpen] = useState(false);
  const confirm = useActionConfirmation(locked);
  const [queue] = useState(() => new DraftSaveQueue<AnalysisPreferences, Preferences>(
    (_id, expectedLockVersion, payload) => invoke("save_analysis_preferences", { expectedLockVersion, payload }),
    (payload) => structuredClone(payload),
  ));
  const [settings, setSettings] = useState(() => queue.snapshot());
  const [profile, setProfile] = useState<"standard" | "highAccuracy">("standard");
  const [prompt, setPrompt] = useState("");
  const [jobs, setJobs] = useState<AnalysisJob[]>([]);
  const [versions, setVersions] = useState<VersionState | null>(null);
  const [flow,setFlow]=useState<AnalysisFlow|null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [preview, setPreview] = useState<Preview | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const retryOperations = useRef(new Map<string, string>());
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const mounted = useRef(true);
  const busyRef = useRef(false);
  const vocabularyStamp = useRef<(() => VocabularyStamp) | null>(null);
  const publication = useVersionPublication(meetingId, "analysis", busy || locked, async () => {
    setPreview(null); setError("");
    try { await onHeadChanged(); await refreshRef.current(); }
    catch (cause) { if (mounted.current) setError(`採用・復元は確認済みですが、画面の再読込に失敗しました。変更命令を再送せず状態を再読込してください: ${String(cause)}`); }
  });

  useEffect(() => {
    mounted.current = true;
    const unsubscribe = queue.subscribe(() => setSettings(queue.snapshot()));
    const unregister = registerPendingEdits(() => queue.flush());
    if (isTauri()) void invoke<Preferences>("get_analysis_preferences").then((value) => { if (mounted.current) queue.load(value); }).catch((cause: unknown) => { if (mounted.current) setError(String(cause)); });
    return () => { mounted.current = false; unsubscribe(); unregister(); };
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
    let refreshAgain = false;
    let unlisten: (() => void) | undefined;
    let unlistenVersions: (() => void) | undefined;
    let unlistenFlow: (() => void) | undefined;
    const refresh = (): Promise<void> => {
      if (disposed) return Promise.resolve();
      if (inFlight) { refreshAgain = true; return inFlight; }
      inFlight = (async () => {
        do {
          refreshAgain = false;
          try {
            const [nextJobs, nextVersions,nextFlow] = await Promise.all([
              invoke<AnalysisJob[]>("get_analysis_jobs", { meetingId }),
              invoke<VersionState>("get_analysis_versions", { meetingId }),
              invoke<AnalysisFlow|null>("get_analysis_flow",{meetingId}),
            ]);
            if (disposed) return;
            setJobs(nextJobs); setVersions(nextVersions);setFlow(nextFlow);
          } catch (cause) { if (!disposed) setError(String(cause)); }
        } while (refreshAgain && !disposed);
      })().finally(() => { inFlight = null; });
      return inFlight;
    };
    refreshRef.current = refresh;
    // Events are invalidations. Only authoritative reads can update the display.
    void listen("analysis-job-changed", () => { void refresh(); }).then((stop) => { if (disposed) stop(); else { unlisten = stop; void refresh(); } }).catch((cause: unknown) => { if (!disposed) setError(String(cause)); });
    void listen<string>("analysis-version-changed", (event) => { if (event.payload === meetingId) void refresh(); }).then((stop) => { if (disposed) stop(); else { unlistenVersions = stop; void refresh(); } }).catch((cause: unknown) => { if (!disposed) setError(String(cause)); });
    void listen("analysis-flow-changed",()=>{void refresh();}).then(stop=>{if(disposed)stop();else{unlistenFlow=stop;void refresh();}}).catch(cause=>{if(!disposed)setError(String(cause));});
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 3000);
    return () => { disposed = true; unlisten?.(); unlistenVersions?.();unlistenFlow?.(); window.clearInterval(timer); };
  }, [meetingId]);


  const run = async (action: () => Promise<void>) => {
    if (busyRef.current || locked) return;
    busyRef.current = true; setBusy(true); setError("");
    try { await action(); }
    catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { busyRef.current = false; if (mounted.current) setBusy(false); }
  };
  const updateEngine = (patch: Partial<EnginePreferences>) => queue.edit((value) => ({ ...value, [profile]: { ...value[profile], ...patch } }));
  const saved = async (action: () => Promise<void>) => {
    let ran = false, failed = false, failure: unknown;
    await withSavedEdits(async () => { ran = true; try { await action(); } catch (cause) { failed = true; failure = cause; if (mounted.current) setError(String(cause)); } });
    if (!ran) throw Error("編集の保存を完了できませんでした。入力を保持し、操作は開始していません。");
    if (failed) throw failure;
  };
  const start = () => run(() => saved(async () => {
    const engine = queue.snapshot().payload?.[profile];
    if (!engine || !engineConfigured(engine)) throw new Error("実行ファイルとモデルを設定してください。");
    if (!vocabularyStamp.current) throw new Error("用語リストを読込・保存してから開始してください。");
    const expectedVocabulary = vocabularyStamp.current();
    await invoke("start_transcription", { request: { meetingId, operationId: crypto.randomUUID(), ...engine, profile: profile === "highAccuracy" ? "high_accuracy" : "standard", prompt, expectedVocabulary } });
    await refreshRef.current();
  }));
  const jobAction = (command: string, jobId: string) => run(async () => {
    await invoke(command, { jobId }); await refreshRef.current();
  });
  const retryJob = (job: AnalysisJob) => {
    return run(async () => {
      await confirm({ title: "文字起こしの再試行を確認", target: `会議「${meetingTitle}」 · ジョブ ${job.id}`, confirmLabel: "同じ条件で再試行する", message: `この文字起こしを同じ固定条件で再試行します。保存済み ${job.completedUnits} 区間を再照合して引き継ぎます。現在の設定画面の変更は使用しません。結果は候補として保存し、現在版と編集は変更しません。`, onError: cause => { if (mounted.current) setError(String(cause)); }, action: async () => {
      const operationId = retryOperations.current.get(job.id) ?? crypto.randomUUID();
      retryOperations.current.set(job.id, operationId);
      try {
        await invoke("retry_transcription", { request: { meetingId, jobId: job.id, operationId, expectedSpecHash: job.specHash } });
        retryOperations.current.delete(job.id);
      } finally { await refreshRef.current(); }
      } });
    });
  };
  const cancelJob = (job: AnalysisJob) => run(async () => { await confirm({ title: "文字起こし処理の取消を確認", target: `会議「${meetingTitle}」 · ジョブ ${job.id}`, confirmLabel: "処理を取り消す", message: "解析ジョブを取り消します。録音と現在版は削除しません。", onError: cause => { if (mounted.current) setError(String(cause)); }, action: async () => { await invoke("cancel_analysis_job", { jobId: job.id }); await refreshRef.current(); } }); });
  const openPreview = (summary: VersionSummary, restoring: boolean, opener: HTMLElement) => {
    returnFocus.current = opener;
    return run(() => saved(async () => {
      const state = await invoke<VersionState>("get_analysis_versions", { meetingId });
      assertReviewedVersion(state, summary, restoring);
      const [analysis, draft, current, currentDraft] = await Promise.all([
        invoke<Analysis | null>("get_analysis_preview", { meetingId, analysisId: summary.id }),
        restoring ? invoke<Draft | null>("get_analysis_version_draft", { meetingId, analysisId: summary.id }) : Promise.resolve(null),
        state.currentId ? invoke<Analysis | null>("get_analysis_preview", { meetingId, analysisId: state.currentId }) : Promise.resolve(null),
        state.currentId ? invoke<Draft | null>("get_analysis_version_draft", { meetingId, analysisId: state.currentId }) : Promise.resolve(null),
      ]);
      if (!analysis || analysis.contentHash !== summary.contentHash) throw new Error("プレビューの内容が変化しています。再読込してください。");
      setVersions(state); setPreview({ summary, analysis, draft, current, currentDraft, restoring });
    }));
  };
  const applyPreview = (confirmationOpener: HTMLElement) => run(async () => {
    if (!preview || publication.inFlight) return;
    const state = await invoke<VersionState>("get_analysis_versions", { meetingId });
    assertReviewedVersion(state, preview.analysis, preview.restoring);
    const currentNo = state.versions.find((item) => item.id === state.currentId)?.revisionNo;
    const previousNo = state.versions.find((item) => item.id === state.previousId)?.revisionNo;
    const message = preview.restoring
      ? `直前版 #${preview.summary.revisionNo} へ戻しますか？ 現在版 #${currentNo} は新しい直前版となり、それぞれの保存済み編集を保持します。`
      : `候補 #${preview.summary.revisionNo} を採用しますか？ ${state.currentId ? `現在版 #${currentNo} とその編集は新しい直前版へ移します。` : "初めての現在版になります。"}${state.previousId ? `今の直前版 #${previousNo} はこの画面から復元できなくなります。参照中のデータと保存済み編集は削除しません。` : ""}`;
    await confirm({ opener: confirmationOpener, title: preview.restoring ? "文字起こしの直前版復元を確認" : "文字起こし候補の採用を確認", target: `会議「${meetingTitle}」 · 文字起こし #${preview.summary.revisionNo}`, confirmLabel: preview.restoring ? "直前版へ戻す" : "候補を採用する", message, onError: cause => { if (mounted.current) setError(String(cause)); }, action: () => saved(async () => {
    if (preview.restoring) {
      await publication.apply({ meetingId, kind: "analysis", id: preview.analysis.id, contentHash: preview.analysis.contentHash, revisionNo: preview.summary.revisionNo, meetingLockVersion: state.meetingLockVersion, currentId: state.currentId, restoring: true }, "restore_analysis_version", { meetingId, expectedMeetingLockVersion: state.meetingLockVersion, expectedPreviousId: preview.analysis.id, expectedContentHash: preview.analysis.contentHash, expectedDraft: state.draft });
    } else {
      await publication.apply({ meetingId, kind: "analysis", id: preview.analysis.id, contentHash: preview.analysis.contentHash, revisionNo: preview.summary.revisionNo, meetingLockVersion: state.meetingLockVersion, currentId: state.currentId, restoring: false }, "adopt_analysis_candidate", { request: { meetingId, revisionId: preview.analysis.id, expectedContentHash: preview.analysis.contentHash, expectedMeetingLockVersion: state.meetingLockVersion, expectedDraft: state.draft, confirmReplacingPrevious: !!state.previousId } });
    }
    setPreview(null);
    await onHeadChanged(); await refreshRef.current();
    }) });
  });
  const engine = settings.payload?.[profile];
  const disabled = busy || locked;
  const pending = jobs.some((job) => ["queued", "running", "pausing"].includes(job.state));
  const previous = versions?.versions.find((item) => item.id === versions.previousId);

  const reviewContent = preview && <>
      <div className="card-head"><h2 id="analysis-preview-title">{preview.restoring ? "直前版の復元確認" : "候補版の採用確認"} #{preview.summary.revisionNo}</h2></div>
      <p>比較する発言は同じ位置に対応するとは限りません。話者・時刻は未検証です。内容SHA-256: <code>{preview.analysis.contentHash}</code></p>
      <details><summary>固定された処理条件</summary><pre>{JSON.stringify({ profile: preview.summary.profile, config: preview.summary.config }, null, 2)}</pre></details>
      <div className="preview-columns">{preview.current ? <PreviewText key={`current-${preview.current.id}`} analysis={preview.current} draft={preview.currentDraft} title="現在版" locked={disabled} /> : <section><h3>現在版</h3><p>まだありません。</p></section>}<PreviewText key={preview.analysis.id} analysis={preview.analysis} draft={preview.draft} title={preview.restoring ? "復元対象" : "候補版"} locked={disabled}/></div>
      <IssueStatus label="文字起こし操作" message={error} />
      {publication.status}
      <div className="analysis-actions"><button type="button" disabled={disabled || publication.inFlight} onClick={event => void applyPreview(event.currentTarget)}>{preview.restoring ? "この直前版へ戻す" : "この候補を採用"}</button><span>保存済み編集は版ごとに保持します。</span></div>
    </>;
  const review: ModalReplacement | null = preview ? {
    key: `${preview.analysis.id}:${preview.restoring}`, labelledBy: "analysis-preview-title", className: "analysis-dialog", content: reviewContent,
    blocked: disabled, onDismiss: () => setPreview(null), restoreTarget: () => returnFocus.current,
    dismissLabel: "閉じる", dismissText: "文字起こしプレビューを閉じる",
  } : null;
  const engineContent = <>
      <p>導入済みの whisper.cpp 実行ファイルと多言語モデルの絶対パスを指定してください。モデル品質・速度は未検証です。開始時にファイルのハッシュを固定します。</p>
      <div className="info-field"><div className="heading-with-info"><label htmlFor={profileSelectId}>プロファイル</label><InfoPopover label="文字起こしの標準／高精度">標準と高精度は、別々に保存したモデルと数値設定の組です。開始時に条件とファイルのハッシュを固定します。品質・速度は未検証で、高精度の負荷は選んだモデルに依存します。</InfoPopover></div><select id={profileSelectId} value={profile} disabled={disabled} onChange={(event) => setProfile(event.target.value as typeof profile)}><option value="standard">標準</option><option value="highAccuracy">高精度</option></select></div>
      {engine ? <fieldset disabled={disabled}>
        <label>whisper-cli の絶対パス<input value={engine.executablePath} maxLength={4096} onChange={(event) => updateEngine({ executablePath: event.target.value })} placeholder="/absolute/path/whisper-cli" /></label>
        <label>多言語モデルの絶対パス<input value={engine.modelPath} maxLength={4096} onChange={(event) => updateEngine({ modelPath: event.target.value })} placeholder="/absolute/path/ggml-model.bin" /></label>
        <label>VADモデルの絶対パス（任意）<input value={engine.vadModelPath ?? ""} maxLength={4096} onChange={(event) => updateEngine({ vadModelPath: event.target.value || null })} /></label>
        <label>CPUスレッド数<input type="number" min={1} max={64} value={engine.threads} onChange={(event) => updateEngine({ threads: Number(event.target.value) })} /></label>
        <label className="inline-label"><input type="checkbox" checked={engine.useGpu} onChange={(event) => updateEngine({ useGpu: event.target.checked })} />GPUを使用（対応ビルドが必要）</label>
      </fieldset> : <p>設定を読込中です。</p>}
      <div className="analysis-actions"><span role="status">{settings.error ? "設定の保存に失敗" : settings.saving ? "設定を保存中" : settings.dirty ? "設定は未保存" : "設定は保存済み"}</span><button type="button" className="small-secondary" disabled={disabled || settings.saving || !settings.dirty} onClick={() => void run(() => queue.retry())}>{settings.error ? "設定保存を再試行" : "設定を保存"}</button></div>
      <IssueNotice scope={meetingId} type="analysis-settings" label="文字起こし設定の保存" message={settings.error} toastMessage="文字起こし設定を保存できません。入力は保持しています" action={{ kind: "retry", label: "文字起こし設定保存を再試行", disabled: disabled || settings.saving, run: () => run(() => queue.retry()) }} />
  </>;
  // Stable secondary fields: hiding or visiting a review must not recreate
  // these inputs, their selection, or their save controller.
  const persistentViews = <>
    <div className="analysis-engine-view" hidden={!engineOpen || !!preview}>
      <h2 id={engineTitleId}>エンジン設定（端末内に保存）</h2>
      <div className="options-dialog-content">{engineContent}</div>
    </div>
    {reviewContent}
  </>;
  const visibleView = review ?? (engineOpen ? {
    key: "analysis-engine", labelledBy: engineTitleId, content: null,
    className: "analysis-engine-dialog", blocked: disabled,
    onDismiss: () => setEngineOpen(false), restoreTarget: () => engineTrigger.current,
    dismissLabel: "閉じる" as const,
  } : null);
  const tools = (vocabularyEntry: ReactNode) => <section className="analysis-card" aria-label="解析操作">
    <AnalysisFlowStatus flow={flow} recorded={recorded} disabled={disabled||recordingActive} versions={versions?.versions??[]} start={()=>void run(()=>saved(async()=>{await invoke("start_analysis_flow",{meetingId});await refreshRef.current();}))} resume={f=>void run(()=>saved(async()=>{await invoke("continue_analysis_flow",{meetingId,expectedLockVersion:f.lockVersion});await refreshRef.current();}))} review={(summary,opener)=>void openPreview(summary,false,opener)}/>
    <div className="card-head"><div className="heading-with-info"><h2>ローカル文字起こし</h2><InfoPopover label="ローカル文字起こし">結果は候補として保存し、採用まで現在版と編集内容を変更しません。話者分離は採用済み文字起こしから開始します。トラック間の時刻補正は未検証です。</InfoPopover></div><IconButton label="状態を再読込" icon="refresh" type="button" className="small-secondary" disabled={disabled} onClick={() => void run(() => refreshRef.current())} /></div>
    {renderTools ? <div className="options-modal analysis-settings"><button ref={engineTrigger} type="button" className="small-secondary options-modal-trigger" aria-label="エンジン設定（端末内に保存）" aria-haspopup="dialog" aria-expanded={engineOpen} onClick={event=>{event.currentTarget.focus({preventScroll:true});setEngineOpen(true);}}>エンジン設定（端末内に保存）</button></div>
      : <OptionsModal className="analysis-settings" label="エンジン設定（端末内に保存）">{engineContent}</OptionsModal>}
    {vocabularyEntry}
    <label className="prompt-label">今回の用語ヒント（認識効果は未検証）<textarea value={prompt} maxLength={8000} disabled={disabled} onChange={(event) => setPrompt(event.target.value)} placeholder="例：点検日。自動置換ではなくエンジンへのヒントです。" /></label>
    <div className="analysis-actions"><button type="button" disabled={disabled || !recorded || recordingActive || pending || !engine || !engineConfigured(engine)} onClick={() => void start()}>文字起こしを開始</button><span>{profile === "highAccuracy" ? "高精度" : "標準"} · {recordingActive ? "録音を優先して解析を停止します" : "端末内で処理"}</span></div>
    {!recorded && <p>保存済み録音が必要です。音声ファイルの取り込みは今後接続します。</p>}
    {jobs.some((job) => job.kind === "transcription") && <ul className="analysis-jobs" aria-label="解析ジョブ">{jobs.filter((job) => job.kind === "transcription").map((job) => <li key={job.id}><div><strong>{jobLabel(job.state)}{job.attempt > 1 ? ` · 試行 ${job.attempt}` : ""}</strong><span>{job.kind} · 保存済み {job.completedUnits}{job.totalUnits === null ? "" : ` / ${job.totalUnits}`} 区間{job.pauseReason === "recording" ? " · 録音・取り込み優先で停止" : ""}</span><small>ジョブ {job.id.slice(0, 8)}{job.retryOfJobId ? ` · 再試行元 ${job.retryOfJobId.slice(0, 8)}` : ""}</small>{job.errorCode && <span className="action-error">{job.errorCode}</span>}</div><div className="analysis-actions">
      {["queued", "running"].includes(job.state) && <IconButton label="一時停止" icon="pause" disabled={disabled} onClick={() => void jobAction("pause_analysis_job", job.id)} />}
      {["paused", "interrupted"].includes(job.state) && <button type="button" className="small-secondary" disabled={disabled || recordingActive || pending} onClick={() => void jobAction("resume_analysis_job", job.id)}>再開</button>}
      {["queued", "running", "paused", "interrupted"].includes(job.state) && <button type="button" className="small-secondary" disabled={disabled} onClick={() => void cancelJob(job)}>取消</button>}
      {canRetryJob(job, jobs) && <button type="button" className="small-secondary" disabled={disabled || recordingActive || pending} onClick={() => void retryJob(job)}>同じ条件で再試行</button>}
      <JobModelRebinding meetingId={meetingId} job={job} jobs={jobs} locked={disabled} recordingActive={recordingActive} withSavedEdits={withSavedEdits} refresh={() => refreshRef.current()} />
    </div></li>)}</ul>}
    <div ref={localDiarization} />
    <div className="analysis-versions"><h3>確認待ちの候補</h3>{versions?.versions.filter((item) => item.publicationState === "candidate").map((item) => <div className="version-row" key={item.id}><span>候補 #{item.revisionNo} · {item.utteranceCount} 発言{item.config.diarization==="sherpa-onnx"?" · 話者分離":""}</span><button type="button" className="small-secondary" disabled={disabled} onClick={(event) => void openPreview(item, false, event.currentTarget)}>内容を確認</button></div>)}{versions && !versions.versions.some((item) => item.publicationState === "candidate") && <p>確認待ちの候補はありません。</p>}{previous && <div className="version-row"><span>直前版 #{previous.revisionNo}</span><button type="button" className="small-secondary" disabled={disabled} onClick={(event) => void openPreview(previous, true, event.currentTarget)}>直前版を確認・復元</button></div>}</div>
    <IssueNotice scope={meetingId} type="analysis-operation" label="文字起こし操作" message={error} toastMessage="文字起こし操作を完了できません。状態を確認してください" />
    {!preview && publication.status}
  </section>;
  if (renderTools) return <DiarizationPanel meetingId={meetingId} meetingTitle={meetingTitle} recorded={recorded} recordingActive={recordingActive} locked={disabled} jobs={jobs} versions={versions} withSavedEdits={withSavedEdits} refresh={()=>refreshRef.current()} visible={diarizationVisible}
    renderSettings={(diarizationEntry,diarizationView,diarizationBody)=><><VocabularyEditor meetingId={meetingId} meetingTitle={meetingTitle} locked={disabled} stampRef={vocabularyStamp} renderEditor={(entry, vocabularyView, vocabularyBody)=>renderTools(tools(entry), visibleView ?? diarizationView ?? vocabularyView, <>{persistentViews}{diarizationBody}{vocabularyBody}</>)} />{createPortal(diarizationEntry,diarizationMount)}</>} />;
  return <>{tools(<VocabularyEditor meetingId={meetingId} meetingTitle={meetingTitle} locked={disabled} stampRef={vocabularyStamp} />)}{createPortal(<DiarizationPanel meetingId={meetingId} meetingTitle={meetingTitle} recorded={recorded} recordingActive={recordingActive} locked={disabled} jobs={jobs} versions={versions} withSavedEdits={withSavedEdits} refresh={()=>refreshRef.current()} visible={diarizationVisible} />,diarizationMount)}<CommonModal ref={dialog} open={!!preview} blocked={disabled} onDismiss={() => setPreview(null)} dismissLabel="閉じる" restoreReady={!disabled} restoreTarget={() => returnFocus.current} className="analysis-dialog" aria-labelledby="analysis-preview-title">{reviewContent}</CommonModal></>;
}
