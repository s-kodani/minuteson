import { createPortal } from "react-dom";
import { IconButton } from "./action-icon";
import { invoke, isTauri } from "./ipc.ts";
import { subscribeInvalidations } from "./native-snapshots";
import { observeInvalidations } from "./snapshot-resource";
import { browserFocus, browserSchedule } from "./snapshot-browser";
import { useEffect, useRef, useState } from "react";
import { flushPendingEdits, TranscriptOperationBusy } from "./draft-save";
import { IssueNotice } from "./toast";
import type { AnalysisFlow } from "./analysis-flow-state";
import { useActionConfirmation } from "./confirmation-service";
import { SavedEditsCancelled, withConfirmedEdits } from "./confirmed-action";
import type { RegenerationRequest, RegenerationTarget } from "./transcription-regeneration";
import { transcriptionAdmissions } from "./transcription-admission";
import { LoadingOverlay } from "./loading-overlay";
import { errorMessage } from "./app-error";

type MeetingRead = <T>(step: "get_meeting" | "list_meetings", work: () => Promise<T>) => Promise<T>;

/** Meeting actions contain no model settings, candidate review or adoption.
 * Native recording/import completion starts the global-settings flow.
 */
export function AnalysisPanel({ meetingId, meetingTitle, hasTranscript = false, recorded, audioDeleted, recordingActive, locked, visible, onHeadChanged, withSavedEdits, onReplaceGenerated, publicationReadStatus, onGenerationStarted, actionsHost }: {
  meetingId: string; meetingTitle: string; hasTranscript?: boolean; recorded: boolean; audioDeleted: boolean | null; recordingActive: boolean; locked: boolean; visible: boolean;
  onHeadChanged: (read: MeetingRead) => Promise<void>; withSavedEdits: (action: () => void | Promise<void>) => Promise<void | "cancelled">;
  onReplaceGenerated: (flow: AnalysisFlow) => Promise<void>;
  publicationReadStatus?: string;
  onGenerationStarted?: () => void;
  actionsHost?: HTMLElement | null;
}) {
  const [flow, setFlow] = useState<AnalysisFlow | null>(null);
  const [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [refreshError, setRefreshError] = useState("");
  const [flowHistoryOnly, setFlowHistoryOnly] = useState(true);
  const [numSpeakers, setNumSpeakers] = useState<number|null>(null);
  const completed = useRef<string | null>(null), running = useRef(false);
  const regeneration = useRef<RegenerationRequest | null>(null);
  const sourceAvailable = useRef(audioDeleted === false); sourceAvailable.current = audioDeleted === false;
  const sourceReason = audioDeleted === true ? "音声は削除済みです。" : audioDeleted === null ? "音声の状態を確認しています。" : undefined;
  const confirm = useActionConfirmation(false);
  const alive = useRef(true);
  const refreshRef = useRef(async () => {});
  useEffect(() => {
    alive.current = true;
    setNumSpeakers(null);
    if (!isTauri()) return;
    let disposed = false;
    let previousFlow: string | undefined;
    const failures = new Map<string, string>();
    const diagnose = async <T,>(
      step: Parameters<MeetingRead>[0] | "subscribe_analysis_events" | "get_analysis_flow" | "flush_pending_transcript_edits",
      work: () => Promise<T>,
    ): Promise<T> => {
      try {
        const value = await work();
        failures.delete(step);
        return value;
      } catch (cause) {
        const message = (cause instanceof Error ? cause.message : errorMessage(cause)).trim().slice(0, 1024) || "詳細なし";
        if (!disposed && failures.get(step) !== message) {
          failures.set(step, message);
          void invoke("record_meeting_open_failure", { meetingId, step, message }).catch(() => {});
        }
        throw cause;
      }
    };
    const resource = observeInvalidations({
      read: async () => {
          const next = await diagnose("get_analysis_flow", () => invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId }));
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
            const ready = await diagnose("flush_pending_transcript_edits", async () => {
              try { await flushPendingEdits("transcription"); return true; }
              catch (cause) {
                if (cause instanceof TranscriptOperationBusy) return false;
                throw cause;
              }
            });
            if (disposed) return;
            if (!ready) {
              // The read succeeded. Retry the pending output on the next refresh
              // without remounting an editor that is still applying an operation.
              setRefreshError("");
              return;
            }
            await onHeadChanged(diagnose);
            if (disposed) return;
            completed.current = output;
          }
          // A recovered read clears only read failures, never an action error.
          // Otherwise a late failed poll survives a successful new generation.
          setRefreshError("");
      },
      subscribe: notify => diagnose("subscribe_analysis_events", () => subscribeInvalidations(["analysis-flow-changed", "analysis-job-changed"], notify)),
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
    if (!sourceAvailable.current) return;
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
  const regenerate = (initial = false) => run(async () => {
    if (!sourceAvailable.current) return;
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
    const outcome = await confirm({ title: initial ? "文字起こしを生成" : "文字起こしを再生成", target: `会議「${meetingTitle}」`,
      message: initial ? "全体設定のモデルで文字起こしと話者分離を生成します。" : "全体設定のモデルで文字起こしと話者分離をやり直します。成功すると現在の本文・話者の修正を置き換えます。処理中に編集した場合は、置き換える前に確認します。失敗した場合は現在の内容を保持します。議事録は変更しません。",
      speakerCount: numSpeakers, onSpeakerCountChange: setNumSpeakers,
      confirmLabel: initial ? "文字起こしを生成" : "文字起こしを再生成", validate: () => alive.current && sourceAvailable.current,
      action: async (chosenSpeakers = null) => {
        const request = { meetingId, operationId: crypto.randomUUID(), expected, numSpeakers: chosenSpeakers };
        regeneration.current = request;
      },
    });
    // Finish the confirmation before admission so a slow IPC response cannot
    // keep its native modal open. run() still owns errors and duplicate guards.
    if (outcome === "completed" && regeneration.current) await submitRegeneration(regeneration.current);
  });
  const jobAction = (command: string) => {
    if (!active || command === "resume_analysis_job" && !sourceAvailable.current) return;
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
      {!resumeSpeakerHandoff && !hasTranscript && (!flow || ["configuration", "attention"].includes(flow.stage) && !flow.replacementRequested) && <button type="button" title={sourceReason} disabled={locked || busy || recordingActive || !recorded || audioDeleted !== false} onClick={() => void regenerate(true)}>文字起こしを生成</button>}
      {(hasTranscript || flow?.stage === "review" || flow?.replacementRequested || regeneration.current) && <button type="button" title={sourceReason} disabled={locked || busy || recordingActive || !recorded || pending || audioDeleted !== false} onClick={() => void regenerate()}>文字起こしを再生成</button>}
      {flow?.replacementReady && <button type="button" disabled={locked || busy || recordingActive} onClick={() => void onReplaceGenerated(flow)}>生成結果に置き換える</button>}
      {resumeSpeakerHandoff && <button type="button" title={sourceReason} disabled={locked || busy || recordingActive || audioDeleted !== false} onClick={() => void run(async () => { if (sourceAvailable.current) await invoke("continue_analysis_flow", { meetingId, expectedLockVersion: flow!.lockVersion }); })}>話者分離を再開</button>}
      {pending && active && ["queued", "running", "pausing", "paused", "interrupted"].includes(active.state) && <div className="analysis-actions">
        {["queued", "running"].includes(active.state) && <IconButton label="生成を一時停止" icon="pause" disabled={locked || busy} onClick={() => jobAction("pause_analysis_job")} />}
        {["paused", "interrupted"].includes(active.state) && <button type="button" title={sourceReason} disabled={locked || busy || recordingActive || audioDeleted !== false} onClick={() => jobAction("resume_analysis_job")}>生成を再開</button>}
        <IconButton label="生成を取り消す" icon="close" disabled={locked || busy} onClick={cancelJob} />
      </div>}
  </>;
  return <>
    <IssueNotice scope={meetingId} type="analysis-operation" label="文字起こし" message={error || flow?.errorMessage} historyOnly={!error && flowHistoryOnly} />
    <LoadingOverlay active={visible && !!refreshError} label="文字起こしの状態を確認中です。" />
    {visible && (flow?.readyToPublish || flow?.replacementReady) && publicationReadStatus && <p role="status">{publicationReadStatus}</p>}
    {actionsHost ? createPortal(actions, actionsHost) : <div className="analysis-actions">{actions}</div>}
  </>;
}
