import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { AnalysisFlow } from "./analysis-flow-state";
import { SavedEditsCancelled } from "./confirmed-action";
import { confirmations, useActionConfirmation } from "./confirmation-service";
import { publishWithReplacementConfirmation, type Publication } from "./transcription-regeneration";
import { toasts } from "./toast";
import { toastKey } from "./toast-state";
import { backgroundReadFailure } from "./background-read-failure";

/** Publication is coordinated even when another meeting is open. A changed
 * draft asks for consent and keeps the completed result available on cancel.
 */
export function useAnalysisPublication(locked: boolean, saveThenAct: (action: () => Promise<void>) => Promise<void | "cancelled">) {
  const [readStatus, setReadStatus] = useState("");
  const confirm = useActionConfirmation(false);
  const live = useRef({ locked, saveThenAct, confirm }); live.current = { locked, saveThenAct, confirm };
  const request = useRef<(flow: AnalysisFlow) => Promise<void>>(async () => {});
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false, running = false;
    const readFailure = backgroundReadFailure("transcription_publication");
    const handled = new Set<string>();
    const failures = new Map<string, { flowId: string; message: string }>();
    const stops: (() => void)[] = [];
    const publish = async (flow: AnalysisFlow) => {
      const key = toastKey(flow.meetingId, "analysis-operation");
      try {
        let deferred = false;
        await publishWithReplacementConfirmation(flow, {
          saveThenAct: action => live.current.saveThenAct(action),
          publish: (current, confirmedTarget) => invoke<Publication>("publish_regenerated_transcription", {
            meetingId: current.meetingId, flowId: current.id, expectedLockVersion: current.lockVersion, confirmedTarget,
          }),
          confirm: options => {
            // Another action may have opened a dialog while the edit was saved.
            // Leave this result for the next poll instead of treating that as consent.
            if (disposed || live.current.locked || confirmations.getSnapshot()) { deferred = true; return Promise.resolve("unconfirmed"); }
            return live.current.confirm({ ...options, validate: () => !disposed && !live.current.locked });
          },
        });
        if (!deferred) handled.add(`${flow.id}:${flow.lockVersion}`);
        const prior = failures.get(flow.meetingId);
        if (prior) toasts.resolveMatching(key, prior.message);
        failures.delete(flow.meetingId);
      } catch (cause) {
        if (disposed || cause instanceof SavedEditsCancelled) return;
        handled.add(`${flow.id}:${flow.lockVersion}`);
        const message = `文字起こし: ${String(cause)}`;
        failures.set(flow.meetingId, { flowId: flow.id, message });
        toasts.show({ key, meetingId: flow.meetingId, level: "error", message });
      }
    };
    request.current = async flow => {
      if (disposed || running || live.current.locked || confirmations.getSnapshot()) return;
      running = true;
      try { await publish(flow); } finally { running = false; }
    };
    const refresh = async () => {
      if (disposed || running || live.current.locked || confirmations.getSnapshot()) return;
      running = true;
      try {
        const flows = await invoke<AnalysisFlow[]>("get_ready_analysis_flows");
        if (disposed) return;
        if (!Array.isArray(flows) || flows.some(flow => !flow || typeof flow.id !== "string"
          || typeof flow.meetingId !== "string" || !Number.isSafeInteger(flow.lockVersion))) {
          throw Error("反映待ちの文字起こしの一覧を確認できません。");
        }
        readFailure.recovered();
        setReadStatus("");
        for (const [meetingId, failure] of failures) {
          try {
            const current = await invoke<AnalysisFlow | null>("get_analysis_flow", { meetingId });
            if (disposed) return;
            if (current?.meetingId === meetingId && (current.id !== failure.flowId || current.stage === "review" && !current.errorMessage)) {
              toasts.resolveMatching(toastKey(meetingId, "analysis-operation"), failure.message);
              failures.delete(meetingId);
            }
          } catch { /* A failed read is not proof that the old issue recovered. */ }
        }
        for (const flow of flows) {
          if (disposed || live.current.locked || confirmations.getSnapshot()) break;
          if (!handled.has(`${flow.id}:${flow.lockVersion}`)) await publish(flow);
        }
      } catch (cause) {
        if (!disposed && readFailure.failed(cause)) {
          setReadStatus("反映待ちの文字起こしを確認できません。自動で再確認しています。");
        }
      }
      finally { running = false; }
    };
    for (const event of ["analysis-flow-changed", "analysis-job-changed"]) {
      void listen(event, () => { void refresh(); }).then(stop => { if (disposed) stop(); else stops.push(stop); }).catch(() => { /* Periodic reads still run. */ });
    }
    void refresh(); const timer = window.setInterval(() => void refresh(), 3000);
    return () => { disposed = true; request.current = async () => {}; stops.forEach(stop => stop()); window.clearInterval(timer); };
  }, []);
  return { readStatus, replace: (flow: AnalysisFlow) => request.current(flow) };
}
