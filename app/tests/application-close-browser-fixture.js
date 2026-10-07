// Synthetic IPC only: production MainWindow, no device or audio access.
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { registerPendingEdits } from "../src/draft-save.ts";
window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const data = window.dataFixture;
const clone = value => structuredClone(value);
const recording = { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null };
const importing = { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, fileName: null, info: null, sourceBytes: null, requiredBytes: null, availableBytes: null, completed: 0, saved: null, errorMessage: null };
const f = window.closeFixture = {
  recording, importing, activity: { analysisBusy: false, modelPreparationBusy: false }, calls: [],
  failFlush: false, flushDelay: 0, flushes: 0, mutateDuringFlush: null,
  startLost: false, cancelImportLost: false, cancelDataLost: false, finishLost: false, failRead: false,
  request: quit => emit("main-close-requested", { quit }),
  recordChanged: async patch => { f.recording = { ...f.recording, ...patch, revision: f.recording.revision + 1 }; await emit("session-changed", clone(f.recording)); },
  importChanged: async patch => { f.importing = { ...f.importing, ...patch, revision: f.importing.revision + 1 }; await emit("audio-import-changed", clone(f.importing)); },
  finishImport: saved => f.importChanged({ busy: false, phase: saved ? "saved" : "cancelled", saved: saved ? { id: crypto.randomUUID(), meetingId: data.meetings[0].id, state: "saved" } : null }),
  finishData: published => data.changed({ busy: false, phase: published ? "completed" : "cancelled", retryAllowed: false, backupPreview: null, backupReceipt: published ? { archiveId: crypto.randomUUID(), meetingCount: 1, assetCount: 0, omittedPcmCount: 0, omittedLogCount: 0, deletedAudioMeetingCount: 0, deletedAudioAssetCount: 0, size: 1024, sha256: "a".repeat(64) } : null }),
};
registerPendingEdits(async () => {
  f.flushes++;
  if (f.flushDelay) await new Promise(resolve => setTimeout(resolve, f.flushDelay));
  if (f.failFlush) throw Error("模擬編集保存失敗・入力を保持");
  if (f.mutateDuringFlush === "import") await f.importChanged({ token: crypto.randomUUID() });
  if (f.mutateDuringFlush === "meeting") { data.meetings[0].title = "確認後に変更された会議"; data.meetings[0].lockVersion++; }
  f.mutateDuringFlush = null;
});
mockIPC(async (command, args) => {
  f.calls.push({ command, args: clone(args) });
  if (command === "get_recording_snapshot") return clone(f.recording);
  if (command === "capture_available") return true;
  if (command === "get_audio_import") { if (f.failRead) throw Error("模擬終了対象読戻し失敗"); return clone(f.importing); }
  if (command === "get_application_close_activity") return clone(f.activity);
  if (command === "cancel_audio_import") {
    if (args.token !== f.importing.token) throw Error("模擬取り込み対象変更");
    await f.importChanged({ phase: f.importing.busy ? "cancelling" : "cancelled" });
    if (f.cancelImportLost) throw Error("模擬取消受付の応答喪失");
    return clone(f.importing);
  }
  if (command === "cancel_data_operation") {
    if (args.token !== data.snapshot.token) throw Error("模擬データ対象変更");
    await data.changed({ phase: data.snapshot.busy ? "cancelling" : "cancelled", retryAllowed: false });
    if (f.cancelDataLost) throw Error("模擬データ取消受付の応答喪失");
    return clone(data.snapshot);
  }
  if (command === "start_recording") {
    if (args.meetingId !== data.meetings[0].id || !["idle", "error"].includes(f.recording.phase)) throw Error("模擬録音対象変更");
    f.recording = { ...f.recording, revision: f.recording.revision + 1, phase: "preparing", meetingId: args.meetingId, sessionId: crypto.randomUUID() };
    await emit("session-changed", clone(f.recording));
    if (f.startLost) throw Error("模擬録音開始受付の応答喪失");
    return null;
  }
  if (command === "finish_main_close") {
    // Mirror native last-hop guards without simulating process exit.
    if (args.expected && (args.expected.importToken !== f.importing.token || args.expected.dataToken !== data.snapshot.token)) throw Error("模擬終了対象変更");
    if (f.finishLost) throw Error("模擬終了受付の応答喪失");
    return null;
  }
  if (["get_analysis_flow", "get_current_analysis", "get_transcript_draft", "get_current_minutes", "get_minutes_draft"].includes(command)) return null;
  if (["get_analysis_jobs", "get_minutes_exports"].includes(command)) return [];
  if (["get_analysis_versions", "get_minutes_versions"].includes(command)) return { meetingLockVersion: data.meetings[0].lockVersion, currentId: null, previousId: null, draft: null, versions: [] };
  if (command === "get_meeting_vocabulary") return { global: clone(data.book), meeting: { id: `meeting:${args.meetingId}`, payload: [], lockVersion: 0 } };
  if (command === "get_diarization_preferences") {
    const engine = { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [], threads: 4, clusterThresholdMilli: 500, windowShiftMilli: 100, minDurationOnMs: 0, minDurationOffMs: 0 };
    return { id: "sherpa_profiles", lockVersion: 0, payload: { standard: clone(engine), highAccuracy: clone(engine) } };
  }
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
