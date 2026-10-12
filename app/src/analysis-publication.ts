import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { AnalysisFlow } from "./analysis-flow-state";
import { withConfirmedEdits } from "./confirmed-action";
import { toasts } from "./toast";
import { toastKey } from "./toast-state";

/** The main window coordinates publication even if this meeting is not open.
 * Its brief edit barrier saves open fields; native compare-and-swap rejects
 * changes since confirmation. The worker never clobbers unsaved UI input.
 */
export function useAnalysisPublication(locked: boolean, saveThenAct: (action: () => Promise<void>) => Promise<void>) {
  const [error, setError] = useState("");
  const live = useRef({ locked, saveThenAct }); live.current = { locked, saveThenAct };
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false, running = false;
    const stops: (() => void)[] = [];
    const refresh = async () => {
      if (disposed || running || live.current.locked) return;
      running = true;
      try {
        const flows = await invoke<AnalysisFlow[]>("get_ready_analysis_flows");
        if (!disposed) setError("");
        if (disposed || live.current.locked || !flows.length) return;
        await withConfirmedEdits(live.current.saveThenAct, async () => {
          if (disposed) return;
          for (const flow of flows) {
            // One conflict must not starve another meeting's completed result.
            const key = toastKey(flow.meetingId, "analysis-operation");
            try {
              await invoke("publish_regenerated_transcription", { meetingId: flow.meetingId, flowId: flow.id, expectedLockVersion: flow.lockVersion });
              toasts.resolve(key);
            } catch (cause) {
              // The selected meeting may show the same native failure. Share
              // its notification identity rather than producing a second toast.
              if (!disposed) toasts.show({ key, meetingId: flow.meetingId, level: "error", message: `文字起こし: ${String(cause)}` });
            }
          }
        });
      } catch (cause) { if (!disposed) setError(String(cause)); }
      finally { running = false; }
    };
    for (const event of ["analysis-flow-changed", "analysis-job-changed"]) void listen(event, () => { void refresh(); }).then(stop => { if (disposed) stop(); else stops.push(stop); });
    void refresh(); const timer = window.setInterval(() => void refresh(), 3000);
    return () => { disposed = true; stops.forEach(stop => stop()); window.clearInterval(timer); };
  }, []);
  return error;
}
