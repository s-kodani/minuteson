import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { DraftSaveQueue, registerPendingEdits, type VersionedDraft } from "./draft-save";
import { assertReviewedVersion, canRetryJob, engineConfigured, jobLabel, type AnalysisJob, type AnalysisPreferences, type EnginePreferences, type VersionState, type VersionSummary } from "./analysis-state";
import { timeLabel, type Analysis, type Draft } from "./transcript";
import { VocabularyEditor } from "./vocabulary";
import type { VocabularyStamp } from "./vocabulary-state";
import { DiarizationPanel } from "./diarization-panel";
import { SpeakerReviewBadge,SpeakerReviewDetails,useSpeakerReviews } from "./speaker-review";
import { useTranscriptAudio } from "./transcript-audio";
import { projectTranscript } from "./transcript-state";

type Preferences = VersionedDraft<AnalysisPreferences>;
type Preview = { summary: VersionSummary; analysis: Analysis; draft: Draft | null; current: Analysis | null; currentDraft: Draft | null; restoring: boolean };

function PreviewText({ analysis, draft, title,locked }: { analysis: Analysis; draft: Draft | null; title: string;locked:boolean }) {
  const [page, setPage] = useState(0);
  const payload=draft?.baseAnalysisId===analysis.id?draft.payload:null;
  const view=projectTranscript(analysis,payload);
  const shown=view.utterances.slice(page*50,(page+1)*50);
  const review=useSpeakerReviews(analysis,shown.map(u=>u.id));
  const audio=useTranscriptAudio(analysis.meetingId,locked);
  const labels=new Map(view.speakers.map(s=>[s.id,(s.displayName||s.label)+(payload?.speaker_merges?.some(m=>m.source_id===s.id)?"（統合前の推定ラベル）":"")]));
  return <section className="preview-text" aria-label={title}>
    <h3>{title} {draft && <small>保存済み編集を含む</small>}</h3>
    {analysis.utterances.length === 0 && <p>発言はありません。</p>}
    {audio.controls}
    {review.error&&<p className="action-error" role="alert">推定区間を照合できません: {review.error} <button type="button" disabled={locked} onClick={review.retry}>推定表示を再読込</button></p>}
    {shown.map((item) => {
      const speakers = item.speakerIds.map((id) => {
        const speaker = view.speakers.find((value) => value.id === id);
        return speaker?.displayName || speaker?.label || "話者不明";
      });
      return <article key={item.id}><small>{timeLabel(item.startUs)}–{timeLabel(item.endUs)} · {speakers.join(" / ") || "話者不明"}</small><button type="button" className="small-secondary" disabled={locked||audio.preparing} onClick={()=>void audio.start(analysis.id,item.id)}>この発言を聞く</button><SpeakerReviewBadge review={review.reviews.get(item.id)}/><p>{item.effectiveText}</p><SpeakerReviewDetails analysis={analysis} utteranceId={item.id} startUs={item.startUs} labels={labels} locked={locked||audio.preparing} onPlay={at=>audio.start(analysis.id,item.id,undefined,at)}/></article>;
    })}
    {analysis.utterances.length > 50 && <div className="transcript-pages"><button type="button" disabled={page === 0} onClick={() => setPage(page - 1)}>前へ</button><span>{page + 1} / {Math.ceil(analysis.utterances.length / 50)}</span><button type="button" disabled={(page + 1) * 50 >= analysis.utterances.length} onClick={() => setPage(page + 1)}>次へ</button></div>}
  </section>;
}

export function AnalysisPanel({ meetingId, recorded, recordingActive, locked, withSavedEdits, onHeadChanged }: {
  meetingId: string; recorded: boolean; recordingActive: boolean; locked: boolean;
  withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  onHeadChanged: () => Promise<void>;
}) {
  const [queue] = useState(() => new DraftSaveQueue<AnalysisPreferences, Preferences>(
    (_id, expectedLockVersion, payload) => invoke("save_analysis_preferences", { expectedLockVersion, payload }),
    (payload) => structuredClone(payload),
  ));
  const [settings, setSettings] = useState(() => queue.snapshot());
  const [profile, setProfile] = useState<"standard" | "highAccuracy">("standard");
  const [prompt, setPrompt] = useState("");
  const [jobs, setJobs] = useState<AnalysisJob[]>([]);
  const [versions, setVersions] = useState<VersionState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [preview, setPreview] = useState<Preview | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const retryOperations = useRef(new Map<string, string>());
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const mounted = useRef(true);
  const vocabularyStamp = useRef<(() => VocabularyStamp) | null>(null);

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
    const refresh = (): Promise<void> => {
      if (disposed) return Promise.resolve();
      if (inFlight) { refreshAgain = true; return inFlight; }
      inFlight = (async () => {
        do {
          refreshAgain = false;
          try {
            const [nextJobs, nextVersions] = await Promise.all([
              invoke<AnalysisJob[]>("get_analysis_jobs", { meetingId }),
              invoke<VersionState>("get_analysis_versions", { meetingId }),
            ]);
            if (disposed) return;
            setJobs(nextJobs); setVersions(nextVersions);
          } catch (cause) { if (!disposed) setError(String(cause)); }
        } while (refreshAgain && !disposed);
      })().finally(() => { inFlight = null; });
      return inFlight;
    };
    refreshRef.current = refresh;
    // Events are invalidations. Only authoritative reads can update the display.
    void listen("analysis-job-changed", () => { void refresh(); }).then((stop) => { if (disposed) stop(); else { unlisten = stop; void refresh(); } }).catch((cause: unknown) => { if (!disposed) setError(String(cause)); });
    void listen<string>("analysis-version-changed", (event) => { if (event.payload === meetingId) void refresh(); }).then((stop) => { if (disposed) stop(); else { unlistenVersions = stop; void refresh(); } }).catch((cause: unknown) => { if (!disposed) setError(String(cause)); });
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 3000);
    return () => { disposed = true; unlisten?.(); unlistenVersions?.(); window.clearInterval(timer); };
  }, [meetingId]);

  useEffect(() => {
    if (!preview || !dialog.current) return;
    const focused = returnFocus.current;
    dialog.current.showModal();
    return () => { dialog.current?.close(); window.setTimeout(() => { if (focused?.isConnected) focused.focus(); }, 0); };
  }, [preview]);

  const run = async (action: () => Promise<void>) => {
    if (busy || locked) return;
    setBusy(true); setError("");
    try { await action(); }
    catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { if (mounted.current) setBusy(false); }
  };
  const updateEngine = (patch: Partial<EnginePreferences>) => queue.edit((value) => ({ ...value, [profile]: { ...value[profile], ...patch } }));
  const saved = (action: () => Promise<void>) => withSavedEdits(async () => {
    try { await action(); }
    catch (cause) { if (mounted.current) setError(String(cause)); throw cause; }
  });
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
    if (!window.confirm(`この文字起こしを同じ固定条件で再試行しますか？ 保存済み ${job.completedUnits} 区間を再照合して引き継ぎます。現在の設定画面の変更は使用しません。結果は候補として保存し、現在版と編集は変更しません。`)) return;
    return run(async () => {
      const operationId = retryOperations.current.get(job.id) ?? crypto.randomUUID();
      retryOperations.current.set(job.id, operationId);
      try {
        await invoke("retry_transcription", { request: { meetingId, jobId: job.id, operationId, expectedSpecHash: job.specHash } });
        retryOperations.current.delete(job.id);
      } finally { await refreshRef.current(); }
    });
  };
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
  const applyPreview = () => run(() => saved(async () => {
    if (!preview) return;
    const state = await invoke<VersionState>("get_analysis_versions", { meetingId });
    assertReviewedVersion(state, preview.analysis, preview.restoring);
    const currentNo = state.versions.find((item) => item.id === state.currentId)?.revisionNo;
    const previousNo = state.versions.find((item) => item.id === state.previousId)?.revisionNo;
    const message = preview.restoring
      ? `直前版 #${preview.summary.revisionNo} へ戻しますか？ 現在版 #${currentNo} は新しい直前版となり、それぞれの保存済み編集を保持します。`
      : `候補 #${preview.summary.revisionNo} を採用しますか？ ${state.currentId ? `現在版 #${currentNo} とその編集は新しい直前版へ移します。` : "初めての現在版になります。"}${state.previousId ? `今の直前版 #${previousNo} はこの画面から復元できなくなります。参照中のデータと保存済み編集は削除しません。` : ""}`;
    if (!window.confirm(message)) return;
    if (preview.restoring) {
      await invoke("restore_analysis_version", { meetingId, expectedMeetingLockVersion: state.meetingLockVersion, expectedPreviousId: preview.analysis.id, expectedContentHash: preview.analysis.contentHash, expectedDraft: state.draft });
    } else {
      await invoke("adopt_analysis_candidate", { request: { meetingId, revisionId: preview.analysis.id, expectedContentHash: preview.analysis.contentHash, expectedMeetingLockVersion: state.meetingLockVersion, expectedDraft: state.draft, confirmReplacingPrevious: !!state.previousId } });
    }
    setPreview(null);
    await onHeadChanged(); await refreshRef.current();
  }));
  const engine = settings.payload?.[profile];
  const disabled = busy || locked;
  const pending = jobs.some((job) => ["queued", "running", "pausing"].includes(job.state));
  const previous = versions?.versions.find((item) => item.id === versions.previousId);

  return <section className="analysis-card" aria-label="解析操作">
    <div className="card-head"><h2>ローカル文字起こし</h2><button type="button" className="small-secondary" disabled={disabled} onClick={() => void run(() => refreshRef.current())}>状態を再読込</button></div>
    <p>結果は候補として保存します。内容を確認して採用するまで、現在版と編集内容は変更しません。話者分離は採用済み文字起こしから下の操作で開始できます。トラック間の時刻補正は未検証です。</p>
    <details className="analysis-settings"><summary>エンジン設定（端末内に保存）</summary>
      <p>導入済みの whisper.cpp 実行ファイルと多言語モデルの絶対パスを指定してください。モデル品質・速度は未検証です。開始時にファイルのハッシュを固定します。</p>
      <label>プロファイル<select value={profile} disabled={disabled} onChange={(event) => setProfile(event.target.value as typeof profile)}><option value="standard">標準</option><option value="highAccuracy">高精度</option></select></label>
      {engine ? <fieldset disabled={disabled}>
        <label>whisper-cli の絶対パス<input value={engine.executablePath} maxLength={4096} onChange={(event) => updateEngine({ executablePath: event.target.value })} placeholder="/absolute/path/whisper-cli" /></label>
        <label>多言語モデルの絶対パス<input value={engine.modelPath} maxLength={4096} onChange={(event) => updateEngine({ modelPath: event.target.value })} placeholder="/absolute/path/ggml-model.bin" /></label>
        <label>VADモデルの絶対パス（任意）<input value={engine.vadModelPath ?? ""} maxLength={4096} onChange={(event) => updateEngine({ vadModelPath: event.target.value || null })} /></label>
        <label>CPUスレッド数<input type="number" min={1} max={64} value={engine.threads} onChange={(event) => updateEngine({ threads: Number(event.target.value) })} /></label>
        <label className="inline-label"><input type="checkbox" checked={engine.useGpu} onChange={(event) => updateEngine({ useGpu: event.target.checked })} />GPUを使用（対応ビルドが必要）</label>
      </fieldset> : <p>設定を読込中です。</p>}
      <div className="analysis-actions"><span role="status">{settings.error ? "設定の保存に失敗" : settings.saving ? "設定を保存中" : settings.dirty ? "設定は未保存" : "設定は保存済み"}</span><button type="button" className="small-secondary" disabled={disabled || settings.saving || !settings.dirty} onClick={() => void run(() => queue.retry())}>{settings.error ? "設定保存を再試行" : "設定を保存"}</button></div>
      {settings.error && <p className="action-error" role="alert">{settings.error} 入力は保持しています。修正後に保存を再試行してください。</p>}
    </details>
    <VocabularyEditor meetingId={meetingId} locked={disabled} stampRef={vocabularyStamp} />
    <label className="prompt-label">今回の用語ヒント（認識効果は未検証）<textarea value={prompt} maxLength={8000} disabled={disabled} onChange={(event) => setPrompt(event.target.value)} placeholder="例：点検日。自動置換ではなくエンジンへのヒントです。" /></label>
    <div className="analysis-actions"><button type="button" disabled={disabled || !recorded || recordingActive || pending || !engine || !engineConfigured(engine)} onClick={() => void start()}>文字起こしを開始</button><span>{profile === "highAccuracy" ? "高精度" : "標準"} · {recordingActive ? "録音を優先して解析を停止します" : "端末内で処理"}</span></div>
    {!recorded && <p>保存済み録音が必要です。音声ファイルの取り込みは今後接続します。</p>}
    {jobs.some((job) => job.kind === "transcription") && <ul className="analysis-jobs" aria-label="解析ジョブ">{jobs.filter((job) => job.kind === "transcription").map((job) => <li key={job.id}><div><strong>{jobLabel(job.state)}{job.attempt > 1 ? ` · 試行 ${job.attempt}` : ""}</strong><span>{job.kind} · 保存済み {job.completedUnits}{job.totalUnits === null ? "" : ` / ${job.totalUnits}`} 区間{job.pauseReason === "recording" ? " · 録音のため停止" : ""}</span><small>ジョブ {job.id.slice(0, 8)}{job.retryOfJobId ? ` · 再試行元 ${job.retryOfJobId.slice(0, 8)}` : ""}</small>{job.errorCode && <span className="action-error">{job.errorCode}</span>}</div><div className="analysis-actions">
      {["queued", "running"].includes(job.state) && <button type="button" className="small-secondary" disabled={disabled} onClick={() => void jobAction("pause_analysis_job", job.id)}>一時停止</button>}
      {["paused", "interrupted"].includes(job.state) && <button type="button" className="small-secondary" disabled={disabled || recordingActive || pending} onClick={() => void jobAction("resume_analysis_job", job.id)}>再開</button>}
      {["queued", "running", "paused", "interrupted"].includes(job.state) && <button type="button" className="small-secondary" disabled={disabled} onClick={() => { if (window.confirm("解析ジョブを取り消しますか？ 録音と現在版は削除しません。")) void jobAction("cancel_analysis_job", job.id); }}>取消</button>}
      {canRetryJob(job, jobs) && <button type="button" className="small-secondary" disabled={disabled || recordingActive || pending} onClick={() => void retryJob(job)}>同じ条件で再試行</button>}
    </div></li>)}</ul>}
    <DiarizationPanel meetingId={meetingId} recorded={recorded} recordingActive={recordingActive} locked={disabled} jobs={jobs} versions={versions} withSavedEdits={withSavedEdits} refresh={()=>refreshRef.current()} />
    <div className="analysis-versions"><h3>確認待ちの候補</h3>{versions?.versions.filter((item) => item.publicationState === "candidate").map((item) => <div className="version-row" key={item.id}><span>候補 #{item.revisionNo} · {item.utteranceCount} 発言{item.config.diarization==="sherpa-onnx"?" · 話者分離":""}</span><button type="button" className="small-secondary" disabled={disabled} onClick={(event) => void openPreview(item, false, event.currentTarget)}>内容を確認</button></div>)}{versions && !versions.versions.some((item) => item.publicationState === "candidate") && <p>確認待ちの候補はありません。</p>}{previous && <div className="version-row"><span>直前版 #{previous.revisionNo}</span><button type="button" className="small-secondary" disabled={disabled} onClick={(event) => void openPreview(previous, true, event.currentTarget)}>直前版を確認・復元</button></div>}</div>
    {error && <p className="action-error" role="alert">{error}</p>}
    {preview && <dialog ref={dialog} className="analysis-dialog" aria-labelledby="analysis-preview-title" onCancel={(event) => { if (disabled) event.preventDefault(); else setPreview(null); }}>
      <div className="card-head"><h2 id="analysis-preview-title">{preview.restoring ? "直前版の復元確認" : "候補版の採用確認"} #{preview.summary.revisionNo}</h2><button type="button" className="small-secondary" disabled={disabled} onClick={() => setPreview(null)}>閉じる</button></div>
      <p>左右の発言は同じ位置に対応するとは限りません。話者・時刻は未検証です。内容SHA-256: <code>{preview.analysis.contentHash}</code></p>
      <details><summary>固定された処理条件</summary><pre>{JSON.stringify({ profile: preview.summary.profile, config: preview.summary.config }, null, 2)}</pre></details>
      <div className="preview-columns">{preview.current ? <PreviewText key={`current-${preview.current.id}`} analysis={preview.current} draft={preview.currentDraft} title="現在版" locked={disabled} /> : <section><h3>現在版</h3><p>まだありません。</p></section>}<PreviewText key={preview.analysis.id} analysis={preview.analysis} draft={preview.draft} title={preview.restoring ? "復元対象" : "候補版"} locked={disabled}/></div>
      {error && <p className="action-error" role="alert">{error}</p>}
      <div className="analysis-actions"><button type="button" disabled={disabled} onClick={() => void applyPreview()}>{preview.restoring ? "この直前版へ戻す" : "この候補を採用"}</button><span>保存済み編集は版ごとに保持します。</span></div>
    </dialog>}
  </section>;
}
