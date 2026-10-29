import { createPortal } from "react-dom";
import { IconButton } from "./action-icon";
import { InfoPopover } from "./info-popover";
import { invoke, isTauri } from "./ipc.ts";
import { subscribeInvalidations } from "./native-snapshots";
import { observeInvalidations } from "./snapshot-resource";
import { browserFocus, browserSchedule } from "./snapshot-browser";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { flushPendingEdits } from "./draft-save";
import { IssueNotice } from "./toast";
import type { ModalReplacement } from "./common-modal";
import type { AnalysisFlow } from "./analysis-flow-state";
import { useActionConfirmation } from "./confirmation-service";
import { SavedEditsCancelled, withConfirmedEdits } from "./confirmed-action";
import type { RegenerationRequest, RegenerationTarget } from "./transcription-regeneration";
import { transcriptionAdmissions } from "./transcription-admission";

/** Meeting actions contain no model settings, candidate review or adoption.
 * Native recording/import completion starts the global-settings flow.
 */
export function AnalysisPanel({ meetingId, meetingTitle, hasTranscript = false, recorded, recordingActive, locked, onHeadChanged, withSavedEdits, onReplaceGenerated, onGenerationStarted, actionsHost, renderTools }: {
  meetingId: string; meetingTitle: string; hasTranscript?: boolean; recorded: boolean; recordingActive: boolean; locked: boolean;
  onHeadChanged: () => Promise<void>; withSavedEdits: (action: () => void | Promise<void>) => Promise<void | "cancelled">;
  onReplaceGenerated: (flow: AnalysisFlow) => Promise<void>;
  onGenerationStarted?: () => void;
  actionsHost?: HTMLElement | null;
  renderTools?: (tools: ReactNode, review: ModalReplacement | null, persistentViews?: ReactNode) => ReactNode;
}) {
  const [flow, setFlow] = useState<AnalysisFlow | null>(null);
  const [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [refreshError, setRefreshError] = useState("");
  const [flowHistoryOnly, setFlowHistoryOnly] = useState(true);
  const [numSpeakers, setNumSpeakers] = useState<number|null>(null);
  const completed = useRef<string | null>(null), running = useRef(false);
  const regeneration = useRef<RegenerationRequest | null>(null);
  const confirm = useActionConfirmation(false);
  const alive = useRef(true);
  const refreshRef = useRef(async () => {});
  useEffect(() => {
    alive.current = true;
    if (!isTauri()) return;
    let disposed = false;
    let previousFlow: string | undefined;
    const resource = observeInvalidations({
      read: async () => {
          const next = await invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId });
          if (disposed) return;
          // The first read is saved history, not a newly failed operation.
          // Only changes observed while this view is open announce a toast.
          const receipt = JSON.stringify([next?.id, next?.lockVersion, next?.errorMessage]);
          if (previousFlow === undefined) setFlowHistoryOnly(true);
          else if (previousFlow !== receipt) setFlowHistoryOnly(false);
          previousFlow = receipt;
          setFlow(next);
          const output = next?.stage === "review" ? next.diarizationJob?.outputAnalysisId : null;
          if (output && completed.current !== output) {
            await flushPendingEdits("transcription");
            await onHeadChanged();
            if (disposed) return;
            completed.current = output;
          }
          // A recovered read clears only read failures, never an action error.
          // Otherwise a late failed poll survives a successful new generation.
          setRefreshError("");
      },
      subscribe: notify => subscribeInvalidations(["analysis-flow-changed", "analysis-job-changed"], notify),
      error: cause => { if (!disposed) setRefreshError(String(cause)); },
      intervalMs: 3000, schedule: browserSchedule, focus: browserFocus,
    });
    refreshRef.current = async () => { await resource.refresh(); };
    return () => { alive.current = false; disposed = true; resource.stop(); };
  }, [meetingId]);
  const run = async (action: () => Promise<void>, allowDuringRecording = false) => {
    if (running.current || locked || recordingActive && !allowDuringRecording) return;
    running.current = true; setBusy(true); setError("");
    try { await withConfirmedEdits(withSavedEdits, action); await refreshRef.current(); }
    catch (cause) { if (!(cause instanceof SavedEditsCancelled)) setError(String(cause)); }
    finally { running.current = false; setBusy(false); }
  };
  const active = flow?.stage === "diarization" ? flow.diarizationJob : flow?.transcriptionJob;
  const pending = !!flow && !flow.replacementReady && ["pending", "transcription", "diarization"].includes(flow.stage);
  const resumeSpeakerHandoff = flow?.stage === "attention" && flow.transcriptionJob?.state === "succeeded" && !flow.diarizationJob;
  const submitRegeneration = async (request: RegenerationRequest) => {
    transcriptionAdmissions.begin(meetingId,meetingTitle,request.operationId);
    onGenerationStarted?.();
    try { await invoke("regenerate_transcription", { request }); regeneration.current = null; }
    catch (cause) {
      transcriptionAdmissions.clear(meetingId,request.operationId);
      try {
        const saved = await invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId });
        if (saved?.id === request.operationId && saved.meetingId === meetingId) {
          regeneration.current = null; return;
        }
        // The pre-confirmation flow still owns this meeting: admission never
        // committed. A later explicit attempt may read a fresh target/model.
        if (saved?.id === request.expected.flow?.id || !saved && !request.expected.flow) regeneration.current = null;
      } catch { /* Unknown acceptance: retain the same intent, never mint another ID. */ }
      throw cause;
    }
    finally {transcriptionAdmissions.end(meetingId,request.operationId);}
  };
  const regenerate = () => run(async () => {
    // A lost acknowledgement retains the original intent and operation ID.
    // Read back the accepted flow before considering any new generation.
    if (regeneration.current) {
      const accepted = await invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId });
      if (accepted?.id === regeneration.current.operationId) {
        regeneration.current = null; return;
      }
      await submitRegeneration(regeneration.current); return;
    }
    const expected = await invoke<RegenerationTarget>("get_transcription_regeneration_target", { meetingId });
    const outcome = await confirm({ title: "文字起こしを再生成", target: `会議「${meetingTitle}」`,
      message: "全体設定のモデルで文字起こしと話者分離をやり直します。成功すると現在の本文・話者の修正を置き換えます。処理中に編集した場合は、置き換える前に確認します。失敗した場合は現在の内容を保持します。議事録は変更しません。" + (numSpeakers ? ` 会議全体で最大${numSpeakers}人。` : ""),
      confirmLabel: "文字起こしを再生成", validate: () => alive.current,
      action: async () => {
        const request = { meetingId, operationId: crypto.randomUUID(), expected, numSpeakers };
        regeneration.current = request;
      },
    });
    // Finish the confirmation before admission so a slow IPC response cannot
    // keep its native modal open. run() still owns errors and duplicate guards.
    if (outcome === "completed" && regeneration.current) await submitRegeneration(regeneration.current);
  });
  const jobAction = (command: string) => {
    if (!active) return;
    void run(async () => { await invoke(command, { jobId: active.id }); }, command !== "resume_analysis_job");
  };
  const cancelJob = () => {
    if (!active) return;
    const jobId = active.id;
    void run(async () => {
      await confirm({ title: "文字起こし生成を取り消す", target: `会議「${meetingTitle}」`,
        message: "文字起こしと話者分離の生成を取り消します。保存済みの文字起こしと編集内容、議事録は保持します。", confirmLabel: "生成を取り消す",
        action: async () => {
          const current = await invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId });
          const currentJob = current?.stage === "diarization" ? current.diarizationJob : current?.transcriptionJob;
          if (current?.id !== flow?.id || currentJob?.id !== jobId || !["queued", "running", "pausing", "paused", "interrupted"].includes(currentJob.state)) {
            throw Error("生成の状態が変わりました。現在の状態を確認して、もう一度操作してください。");
          }
          await invoke("cancel_analysis_job", { jobId });
        },
      });
    }, true);
  };
  const actions = <>
      {!resumeSpeakerHandoff && !hasTranscript && (!flow || ["configuration", "attention"].includes(flow.stage) && !flow.replacementRequested) && <button type="button" disabled={locked || busy || recordingActive || !recorded} onClick={() => void (flow?.stage === "attention" ? regenerate() : run(async()=>{
        const expected=await invoke<RegenerationTarget>("get_transcription_regeneration_target",{meetingId});
        const request={meetingId,operationId:crypto.randomUUID(),expected,numSpeakers};
        regeneration.current=request;await submitRegeneration(request);
      }))}>文字起こしを生成</button>}
      {(hasTranscript || flow?.stage === "review" || flow?.replacementRequested || regeneration.current) && <button type="button" disabled={locked || busy || recordingActive || !recorded || pending} onClick={() => void regenerate()}>文字起こしを再生成</button>}
      {pending && active && ["queued", "running", "pausing", "paused", "interrupted"].includes(active.state) && <div className="analysis-actions">
        {["queued", "running"].includes(active.state) && <IconButton label="生成を一時停止" icon="pause" disabled={locked || busy} onClick={() => jobAction("pause_analysis_job")} />}
        {["paused", "interrupted"].includes(active.state) && <button type="button" disabled={locked || busy || recordingActive} onClick={() => jobAction("resume_analysis_job")}>生成を再開</button>}
        <IconButton label="生成を取り消す" icon="close" disabled={locked || busy} onClick={cancelJob} />
      </div>}
  </>;
  const tools = <section className="analysis-card" aria-label="文字起こし操作">
    <div className="heading-with-info"><h2>文字起こし</h2><InfoPopover label="文字起こし">全体設定のモデルを使います。完了した結果は発言一覧に保存します。</InfoPopover></div>
    {pending ? <p role="status">{flow?.stage === "diarization" ? "話者分離" : "文字起こし"}{!active ? "の開始を準備しています。" : active.state === "paused" ? "を一時停止しています。" : active.state === "interrupted" ? "が中断しています。" : active.state === "queued" ? "の開始を待っています。" : active.state === "pausing" ? "を停止しています。" : "中です。"}進捗は画面上部で確認できます。</p> : (
    <div className="speaker-count-fields"><label>会議全体の話者数<select value={numSpeakers??""} disabled={locked||busy||recordingActive||pending||!!regeneration.current} onChange={e=>setNumSpeakers(e.target.value?Number(e.target.value):null)}><option value="">自動</option>{Array.from({length:100},(_,i)=><option key={i+1} value={i+1}>最大{i+1}人</option>)}</select></label></div>
    )}
    <div className="analysis-actions">
      {flow?.replacementReady && <button type="button" disabled={locked || busy || recordingActive} onClick={() => void onReplaceGenerated(flow)}>生成結果に置き換える</button>}
      {resumeSpeakerHandoff && <button type="button" disabled={locked || busy || recordingActive} onClick={() => void run(() => invoke("continue_analysis_flow", { meetingId, expectedLockVersion: flow!.lockVersion }))}>話者分離を再開</button>}
    </div>
  </section>;
  return <>
    <IssueNotice scope={meetingId} type="analysis-operation" label="文字起こし" message={error || flow?.errorMessage} historyOnly={!error && flowHistoryOnly} />
    {refreshError && <p role="status">文字起こしの状態を確認中です。</p>}
    {actionsHost ? createPortal(actions, actionsHost) : <div className="analysis-actions">{actions}</div>}
    {renderTools ? renderTools(tools, null) : tools}
  </>;
}
