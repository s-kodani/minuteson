// Baseline for older main-window browser fixtures. Not production code.
export function dataIdleRead(command) {
  if (command === "get_data_operation") return { revision: 0, token: null, kind: null, phase: "idle", busy: false, backupPreview: null, restorePreview: null, backupReceipt: null, restoreReceipt: null, recoveryPreview: null, recoveryReceipt: null, errorMessage: null, attentionRequired: false, retryAllowed: false };
  if (command === "get_recovery_items") return { supported: true, root: "/mock", items: [] };
  if (command === "get_data_operation_availability") return { available: true, reason: null };
  if (command === "list_restored_meeting_ids") return [];
}
