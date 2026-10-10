/** Presentation only: hide tools only after a supported, connected read proves
 * the list empty. Unknown state and any operation/attention stay visible. */
export function quietMaintenance(state: {
  compact: boolean; connected: boolean; supported: boolean;
  listed: boolean; itemCount: number; attention: boolean;
}): boolean {
  return state.compact && state.connected && state.supported && state.listed
    && state.itemCount === 0 && !state.attention;
}

// Refresh orphan diagnostics after any data operation reaches a terminal state,
// not only after recovery itself. An idle poll must not repeatedly scan files.
export function maintenanceReadRevision(snapshot: { phase: string; busy: boolean; revision: number } | null): number {
  return snapshot && !snapshot.busy && ["completed", "failed", "cancelled", "attention"].includes(snapshot.phase)
    ? snapshot.revision : -1;
}
