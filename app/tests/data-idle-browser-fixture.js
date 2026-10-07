// Baseline for older main-window browser fixtures. Not production code.
export function dataIdleRead(command) {
  if (command === "get_application_close_activity") return { analysisBusy: false, modelPreparationBusy: false };
  if (command === "get_recording_confirmation") return { revision: 0, id: null, phase: "idle", sessionId: null, meetingId: null, meetingTitle: null, origin: null, preparing: false, quitAfterSave: false, errorMessage: null };
  if (command === "get_ui_preferences") return { id: "ui_preferences", payload: { schemaVersion: 1, theme: "system" }, lockVersion: 0 };
  if (command === "get_data_operation") return { revision: 0, token: null, kind: null, phase: "idle", busy: false, backupPreview: null, restorePreview: null, backupReceipt: null, restoreReceipt: null, recoveryPreview: null, recoveryReceipt: null, relocationPreview: null, relocationReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: false };
  if (command === "get_recovery_items") return { supported: true, root: "/mock", items: [] };
  if (command === "get_data_operation_availability") return { available: true, reason: null };
  if (command === "list_restored_meeting_ids") return [];
}
