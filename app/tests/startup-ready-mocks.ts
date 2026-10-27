// Explicit successful native storage-admission baseline for existing suites.
// No unknown-command catch-all: every other command uses the original mock.
import { mockIPC as nativeMockIPC } from "@tauri-apps/api/mocks";
export { mockWindows } from "@tauri-apps/api/mocks";
export const mockIPC: typeof nativeMockIPC = (callback, options) => nativeMockIPC((command, args) => {
  if (command === "get_startup_report") return { schemaVersion: 1, phase: "ready", failure: null, ...(new URLSearchParams(location.search).has("resident") ? { platform: new URLSearchParams(location.search).get("resident") === "macos" ? "macos" : "windows" } : {}) };
  // Window chrome may mount on Windows before the fixture's workspace providers.
  if (command === "plugin:window|title") return document.title;
  if (command === "plugin:window|is_maximized") return false;
  const windowState=(window as Window & {monWindowState?:{visible:boolean;minimized:boolean}}).monWindowState;
  if (command === "plugin:window|is_minimized") return windowState?.minimized??false;
  if (command === "plugin:window|is_visible") return windowState?.visible??true;
  if (command === "get_recording_recovery" && !(window as Window & { recordingRecoveryFixture?: unknown }).recordingRecoveryFixture) return {runId:"fixture-idle",revision:0,phase:"completed",items:[],error:null};
  return callback(command, args);
}, options);
