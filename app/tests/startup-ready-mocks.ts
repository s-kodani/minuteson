// Explicit successful native storage-admission baseline for existing suites.
// No unknown-command catch-all: every other command uses the original mock.
import { mockIPC as nativeMockIPC } from "@tauri-apps/api/mocks";
export { mockWindows } from "@tauri-apps/api/mocks";
export const mockIPC: typeof nativeMockIPC = (callback, options) => nativeMockIPC((command, args) => command === "get_startup_report" ? { schemaVersion: 1, phase: "ready", failure: null } : callback(command, args), options);
