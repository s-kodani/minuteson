// Explicit successful native storage-admission baseline for existing suites.
// No unknown-command catch-all: every other command uses the original mock.
import { mockIPC as nativeMockIPC } from "@tauri-apps/api/mocks";
import { defaultVadOptions } from "../src/vad-state";
import { getCurrentWindow } from "@tauri-apps/api/window";
export { mockWindows } from "@tauri-apps/api/mocks";
export const mockIPC: typeof nativeMockIPC = (callback, options) => nativeMockIPC((command, args) => {
  if (command === "get_model_download_guidance" && !(window as Window & {lifecycleFixture?:unknown}).lifecycleFixture) return {pending:false};
  if (command === "list_job_history" && !(window as Window & {lifecycleFixture?:unknown}).lifecycleFixture) return {items:[],nextCursor:null,storageError:null};
  if (command === "get_window_environment") return { schemaVersion: 1, productName: "MinutesOn", role: getCurrentWindow().label, platform: new URLSearchParams(location.search).get("resident") ?? (navigator.platform.startsWith("Win") ? "windows" : "macos"), capabilities: { voiceIdentity: true } };
  if (command === "get_voice_audio_storage_state" && !(window as Window & {monVoiceFixture?:unknown}).monVoiceFixture) return {cleanupPending:false};
  if (command === "get_asr_backends") return [{ id: "whisper.cpp", vocabularyDescription: "対応：表記・読み。用語と読みを文章のヒントとして渡します。補足は保存・表示用です。", supportsVadOverlap: true, defaultVad: structuredClone(defaultVadOptions) }];
  if (command === "get_startup_report") return { schemaVersion: 1, phase: "ready", failure: null, ...(new URLSearchParams(location.search).has("resident") ? { platform: new URLSearchParams(location.search).get("resident") === "macos" ? "macos" : "windows" } : {}) };
  // Window chrome may mount on Windows before the fixture's workspace providers.
  if (command === "plugin:window|title") return document.title;
  if (command === "plugin:window|is_maximized") return false;
  const windowState=(window as Window & {monWindowState?:{visible:boolean;minimized:boolean}}).monWindowState;
  if (command === "plugin:window|is_minimized") return windowState?.minimized??false;
  if (command === "plugin:window|is_visible") return windowState?.visible??true;
  if (command === "get_recording_recovery" && !(window as Window & { recordingRecoveryFixture?: unknown }).recordingRecoveryFixture) return {runId:"fixture-idle",revision:0,phase:"completed",items:[],error:null};
  return Promise.resolve().then(() => callback(command, args)).catch(cause => {
    // Legacy synthetic drivers predate the typed native error envelope.
    // Compatibility stays in this fixture transport, never in product logic.
    if (typeof cause === "string") {
      for (const code of ["meeting_deletion_not_started", "audio_deletion_not_started", "model_deletion_not_started"]) {
        if (cause.startsWith(`${code}:`)) throw {code,message:cause.slice(code.length+1),operationId:null,retryDisposition:"not_started"};
      }
    }
    throw cause;
  });
}, options);
