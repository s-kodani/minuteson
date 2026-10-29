import { useRef, useState } from "react";
import { flushSync } from "react-dom";
import { flushPendingEdits } from "./draft-save";
import { runAfterSaving, type SavedActionPhase } from "./saved-action";
import { SavedEditsCancelled } from "./confirmed-action";
import { errorMessage } from "./app-error";

/** Owns the edit barrier across navigation, regeneration and shutdown. */
export function useSavedNavigation(reportError: (message: string) => void) {
  const [phase, setPhase] = useState<SavedActionPhase>("idle");
  const running = useRef(false);
  const withSavedEdits = async (action: () => void | Promise<void>, reason: "leave" | "tab" | "transcription" = "leave") => {
    if (running.current) return;
    running.current = true; reportError("");
    try {
      const reportPhase = (next: SavedActionPhase) => {
        // Regeneration must paint its brief edit lock before native publication.
        if (reason === "transcription") flushSync(() => setPhase(next));
        else setPhase(next);
      };
      await runAfterSaving(() => flushPendingEdits(reason), action, reportPhase);
    } catch (cause) {
      if (cause instanceof SavedEditsCancelled) return "cancelled" as const;
      reportError(`操作を完了できませんでした。編集内容はこの画面に保持しています: ${errorMessage(cause)}`);
    } finally { running.current = false; }
  };
  return { phase, busy: phase !== "idle", running, withSavedEdits };
}
