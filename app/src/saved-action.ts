export type SavedActionPhase = "idle" | "saving" | "working";

// Saving remains a prerequisite. The following operation can take minutes;
// its wait must not be described as a save that has not finished.
export async function runAfterSaving(
  flush: () => Promise<void>,
  action: () => void | Promise<void>,
  report: (phase: SavedActionPhase) => void,
): Promise<void> {
  report("saving");
  try {
    await flush();
    report("working");
    await action();
  } finally {
    report("idle");
  }
}
