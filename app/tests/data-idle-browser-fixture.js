// Baseline for older main-window browser fixtures. Not production code.
export function dataIdleRead(command) {
  if (command === "get_ready_analysis_flows") return [];
  if (command === "get_model_download_catalog") return { supported: false, entries: [], unknownEntries: 0 };
  if (command === "get_recorder_operation_notices") return { schemaVersion: 1, entries: ["window_action", "confirmation", "theme_read", "info_read", "session_read"].map(source => ({ source, revision: 0, eventId: null, message: null, dismissed: false })) };
  if (command === "get_capture_test_recovery") return { schemaVersion: 1, revision: 2, phase: "clear", runId: null, errorMessage: null };
  if (command === "get_capture_test_snapshot") return null;
  if (["register_playback_preparation","cancel_playback_preparation","finish_playback_preparation"].includes(command)) return null;
  if (command === "get_analysis_notification_jobs") return [];
  if (command === "get_archived_recording_input_notices") return {schemaVersion:3,recordings:[],nextCursor:null};
  if (command === "get_application_close_activity") return { analysisBusy: false, modelPreparationBusy: false };
  if (command === "get_recording_confirmation") return { revision: 0, id: null, phase: "idle", sessionId: null, meetingId: null, meetingTitle: null, origin: null, preparing: false, quitAfterSave: false, errorMessage: null };
  if (command === "get_ui_preferences") return { id: "ui_preferences", payload: { schemaVersion: 1, theme: "system" }, lockVersion: 0 };
  if (command === "get_data_operation") return { revision: 0, token: null, kind: null, phase: "idle", busy: false, backupPreview: null, restorePreview: null, backupReceipt: null, restoreReceipt: null, relocationPreview: null, relocationReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: false };
  if (command === "get_data_operation_availability") return { available: true, reason: null };
  if (command === "list_restored_meeting_ids") return [];
}
