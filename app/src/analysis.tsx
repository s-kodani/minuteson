import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { flushPendingEdits } from "./draft-save";
import { IssueNotice } from "./toast";
import type { ModalReplacement } from "./common-modal";
import type { AnalysisFlow } from "./analysis-flow-state";
import { jobLabel, type AnalysisJob } from "./analysis-state";

/** Meeting actions contain no model settings, candidate review or adoption.
 * Native recording/import completion starts the global-settings flow.
 */
export function AnalysisPanel({ meetingId, recorded, recordingActive, locked, onHeadChanged, withSavedEdits, renderTools }: {
  meetingId: string; meetingTitle: string; recorded: boolean; recordingActive: boolean; locked: boolean;
  onHeadChanged: () => Promise<void>; withSavedEdits: (action: () => void | Promise<void>) => Promise<void>;
  renderTools?: (tools: ReactNode, review: ModalReplacement | null, persistentViews?: ReactNode) => ReactNode;
}) {
  const [flow, setFlow] = useState<AnalysisFlow | null>(null), [jobs, setJobs] = useState<AnalysisJob[]>([]);
  const [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const completed = useRef<string | null>(null), running = useRef(false);
  const retryOperations = useRef(new Map<string, string>());
  const refreshRef = useRef(async () => {});
  useEffect(() => {
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
            await flushPendingEdits();
            await onHeadChanged();
            if (disposed) return;
            completed.current = output;
          }
        } catch (cause) { if (!disposed) setError(String(cause)); }
      } while (again && !disposed); })().finally(() => { pending = null; });
      return pending;
    };
    refreshRef.current = refresh;
    for (const event of ["analysis-flow-changed", "analysis-job-changed"]) void listen(event, () => { void refresh(); }).then(stop => { if (disposed) stop(); else stops.push(stop); });
    void refresh(); const timer = window.setInterval(() => void refresh(), 3000);
    return () => { disposed = true; stops.forEach(stop => stop()); window.clearInterval(timer); };
  }, [meetingId]);
  const run = async (action: () => Promise<void>) => {
    if (running.current || locked || recordingActive) return;
    running.current = true; setBusy(true); setError("");
    try { await withSavedEdits(action); await refreshRef.current(); }
    catch (cause) { setError(String(cause)); }
    finally { running.current = false; setBusy(false); }
  };
  const active = flow?.stage === "diarization" ? flow.diarizationJob : flow?.transcriptionJob;
  const pending = !!flow && ["pending", "transcription", "diarization"].includes(flow.stage);
  const action = !flow ? () => invoke<void>("start_analysis_flow", { meetingId }) : () => invoke<void>("continue_analysis_flow", { meetingId, expectedLockVersion: flow.lockVersion });
  const failed = jobs.find(job => job.id === active?.id && ["failed", "cancelled"].includes(job.state));
  const retry = async () => {
    if (!failed) return;
    const operationId = retryOperations.current.get(failed.id) ?? crypto.randomUUID();
    retryOperations.current.set(failed.id, operationId);
    await invoke(failed.kind === "diarization" ? "retry_diarization" : "retry_transcription", { request: { meetingId, jobId: failed.id, operationId, expectedSpecHash: failed.specHash } });
    if (flow?.stage === "attention") await invoke("continue_analysis_flow", { meetingId, expectedLockVersion: flow.lockVersion });
  };
  const tools = <section className="analysis-card" aria-label="文字起こし操作">
    <h2>文字起こし</h2><p>全体設定のモデルを使います。完了した結果は発言一覧に保存します。</p>
    {(!flow || ["configuration", "attention"].includes(flow.stage)) && <button type="button" disabled={locked || busy || recordingActive || !recorded} onClick={() => void run(action)}>{flow ? "文字起こしを再開" : "文字起こしを開始"}</button>}
    {failed && <button type="button" disabled={locked || busy || recordingActive} onClick={() => void run(retry)}>再試行</button>}
    {active && ["paused", "interrupted"].includes(active.state) && <button type="button" disabled={locked || busy || recordingActive} onClick={() => void run(() => invoke("resume_analysis_job", { jobId: active.id }))}>再開</button>}
  </section>;
  return <>
    {pending && <p className="transcript-state" role="status">{flow?.stage === "diarization" ? "話者を整理しています" : "文字起こし中"}{active ? " · " + jobLabel(active.state) : ""}</p>}
    <IssueNotice scope={meetingId} type="analysis-operation" label="文字起こし" message={error || flow?.errorMessage} />
    {renderTools ? renderTools(tools, null) : tools}
  </>;
}
