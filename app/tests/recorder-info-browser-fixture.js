// Actual production window components, synthetic IPC only; no native capture.
import { mockIPC, mockWindows } from "./startup-ready-mocks.ts";
window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const label = new URLSearchParams(location.search).get("window") ?? "main";
mockWindows(label);
mockIPC(async (command, args) => {
  if (["get_ui_preferences", "get_recording_snapshot", "get_recording_confirmation", "get_recorder_info", "show_recorder_info", "close_recorder_info", "get_recorder_operation_notices", "publish_recorder_operation_notice", "dismiss_recorder_operation_notice", "open_main_window"].includes(command)) {
    const response = await fetch("/__synthetic_recorder_info", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ command, args, label }) });
    const result = await response.json(); if (result.error) throw Error(result.error); return result.value;
  }
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
await import("../src/bootstrap.tsx");
