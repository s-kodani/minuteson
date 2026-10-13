import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { flushPendingEdits } from "./draft-save";
import { IssueNotice } from "./toast";
import type { ModalReplacement } from "./common-modal";
import type { AnalysisFlow } from "./analysis-flow-state";
import { jobLabel, type AnalysisJob } from "./analysis-state";
import { useActionConfirmation } from "./confirmation-service";
import { SavedEditsCancelled, withConfirmedEdits } from "./confirmed-action";
import type { RegenerationRequest, RegenerationTarget } from "./transcription-regeneration";

/** Meeting actions contain no model settings, candidate review or adoption.
 * Native recording/import completion starts the global-settings flow.
 */
export function AnalysisPanel({ meetingId, meetingTitle, hasTranscript = false, recorded, recordingActive, locked, onHeadChanged, withSavedEdits, renderTools }: {
  meetingId: string; meetingTitle: string; hasTranscript?: boolean; recorded: boolean; recordingActive: boolean; locked: boolean;
  onHeadChanged: () => Promise<void>; withSavedEdits: (action: () => void | Promise<void>) => Promise<void | "cancelled">;
  renderTools?: (tools: ReactNode, review: ModalReplacement | null, persistentViews?: ReactNode) => ReactNode;
}) {
  const [flow, setFlow] = useState<AnalysisFlow | null>(null), [jobs, setJobs] = useState<AnalysisJob[]>([]);
  const [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [refreshError, setRefreshError] = useState("");
  const completed = useRef<string | null>(null), running = useRef(false);
  const retryOperations = useRef(new Map<string, string>());
  const regeneration = useRef<RegenerationRequest | null>(null);
  const confirm = useActionConfirmation(false);
  const alive = useRef(true);
  const refreshRef = useRef(async () => {});
  useEffect(() => {
    alive.current = true;
    if (!isTauri()) return;
    let disposed = false, pending: Promise<void> | null = null, again = false;
    const stops: (() => void)[] = [];
    const refresh = (): Promise<void> => {
      if (disposed) return Promise.resolve();
      if (pending) { again = true; return pending; }
      pending = (async () => { do {
        again = false;
        try {
          const [next, nextJobs] = await Promise.all([invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId }), invoke<AnalysisJob[]>("get_analysis_jobs", { meetingId })]);
          if (disposed) return;
          setFlow(next); setJobs(nextJobs);
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
        } catch (cause) { if (!disposed) setRefreshError(String(cause)); }
      } while (again && !disposed); })().finally(() => { pending = null; });
      return pending;
    };
    refreshRef.current = refresh;
    for (const event of ["analysis-flow-changed", "analysis-job-changed"]) void listen(event, () => { void refresh(); }).then(stop => { if (disposed) stop(); else stops.push(stop); });
    void refresh(); const timer = window.setInterval(() => void refresh(), 3000);
    return () => { alive.current = false; disposed = true; stops.forEach(stop => stop()); window.clearInterval(timer); };
  }, [meetingId]);
  const run = async (action: () => Promise<void>) => {
    if (running.current || locked || recordingActive) return;
    running.current = true; setBusy(true); setError("");
    try { await withConfirmedEdits(withSavedEdits, action); await refreshRef.current(); }
    catch (cause) { if (!(cause instanceof SavedEditsCancelled)) setError(String(cause)); }
    finally { running.current = false; setBusy(false); }
  };
  const active = flow?.stage === "diarization" ? flow.diarizationJob : flow?.transcriptionJob;
  const pending = !!flow && ["pending", "transcription", "diarization"].includes(flow.stage);
  const action = !flow ? () => invoke<void>("start_analysis_flow", { meetingId }) : () => invoke<void>("continue_analysis_flow", { meetingId, expectedLockVersion: flow.lockVersion });
  const failed = jobs.find(job => job.id === active?.id && ["failed", "cancelled"].includes(job.state));
  const submitRegeneration = async (request: RegenerationRequest) => {
    try { await invoke("regenerate_transcription", { request }); regeneration.current = null; }
    catch (cause) {
      try {
        const saved = await invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId });
        if (saved?.id === request.operationId && saved.meetingId === meetingId && saved.replacementRequested) {
          regeneration.current = null; return;
        }
        // The pre-confirmation flow still owns this meeting: admission never
        // committed. A later explicit attempt may read a fresh target/model.
        if (saved?.id === request.expected.flow?.id || !saved && !request.expected.flow) regeneration.current = null;
      } catch { /* Unknown acceptance: retain the same intent, never mint another ID. */ }
      throw cause;
    }
  };
  const regenerate = () => run(async () => {
    // A lost acknowledgement retains the original intent and operation ID.
    // Read back the accepted flow before considering any new generation.
    if (regeneration.current) {
      const accepted = await invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId });
      if (accepted?.id === regeneration.current.operationId && accepted.replacementRequested) {
        regeneration.current = null; return;
      }
      await submitRegeneration(regeneration.current); return;
    }
    const expected = await invoke<RegenerationTarget>("get_transcription_regeneration_target", { meetingId });
    await confirm({ title: "文字起こしを再生成", target: `会議「${meetingTitle}」`,
      message: "全体設定のモデルで文字起こしと話者分離をやり直します。成功すると現在の本文・話者の修正を置き換えます。失敗した場合や、処理中に編集した場合は現在の内容を保持します。議事録は変更しません。",
      confirmLabel: "文字起こしを再生成", validate: () => alive.current,
      action: async () => {
        const request = { meetingId, operationId: crypto.randomUUID(), expected };
        regeneration.current = request;
        await submitRegeneration(request);
      },
    });
  });
  const retry = async () => {
    if (!failed) return;
    const operationId = retryOperations.current.get(failed.id) ?? crypto.randomUUID();
    retryOperations.current.set(failed.id, operationId);
    await invoke(failed.kind === "diarization" ? "retry_diarization" : "retry_transcription", { request: { meetingId, jobId: failed.id, operationId, expectedSpecHash: failed.specHash } });
    if (flow?.stage === "attention") await invoke("continue_analysis_flow", { meetingId, expectedLockVersion: flow.lockVersion });
  };
  const tools = <section className="analysis-card" aria-label="文字起こし操作">
    <h2>文字起こし</h2><p>全体設定のモデルを使います。完了した結果は発言一覧に保存します。</p>
    {!hasTranscript && (!flow || ["configuration", "attention"].includes(flow.stage) && !flow.replacementRequested) && <button type="button" disabled={locked || busy || recordingActive || !recorded} onClick={() => void run(action)}>{flow ? "文字起こしを再開" : "文字起こしを開始"}</button>}
    {(hasTranscript || flow?.stage === "review" || flow?.replacementRequested || regeneration.current) && <button type="button" disabled={locked || busy || recordingActive || !recorded || pending} onClick={() => void regenerate()}>文字起こしを再生成</button>}
    {failed && <button type="button" disabled={locked || busy || recordingActive} onClick={() => void run(retry)}>再試行</button>}
    {active && ["paused", "interrupted"].includes(active.state) && <button type="button" disabled={locked || busy || recordingActive} onClick={() => void run(() => invoke("resume_analysis_job", { jobId: active.id }))}>再開</button>}
  </section>;
  return <>
    {pending && <p className="transcript-state" role="status">{flow?.stage === "diarization" ? "話者を整理しています" : "文字起こし中"}{active ? " · " + jobLabel(active.state) : ""}</p>}
    <IssueNotice scope={meetingId} type="analysis-operation" label="文字起こし" message={error || refreshError || flow?.errorMessage} />
    {renderTools ? renderTools(tools, null) : tools}
  </>;
}
