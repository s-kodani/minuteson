// Dedicated denied/unknown/ready startup scenarios. Unlike existing suite
// baselines, this uses the unwrapped native mock and production root gate.
import "../src/style.css";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
window.isTauri = true; mockWindows("main");
const variant = new URLSearchParams(location.search).get("fixture") ?? "blocked";
const report = { schemaVersion: 1, phase: "blocked", failure: { stage: "storage_admission", message: "一時領域台帳が不正です。<img src=x onerror=alert(1)>\n元の音声を保持します。", messageTruncated: false, meetingRoot: "/safe/会議保存先", configRoot: "/safe/起動設定" } };
const f = window.startupFixture = { calls: [], report, readFail: variant === "read-fail", quitFail: false, delay: false, pendingRead: null, pendingQuit: null };
f.emitResidency = (event,payload) => emit(event,payload);
if (variant.startsWith("windows")) {
  f.report.platform = "windows";
  f.report.failure.message = "他のMONが同じ保存先を使用しています。終了してから起動し直してください。";
}
if (variant === "ready") f.report = { schemaVersion: 1, phase: "ready", failure: null };
if (variant === "starting" || variant.startsWith("transition-")) f.report = { schemaVersion: 1, phase: "starting", failure: null };
if (variant === "selection") f.report.failure = { ...report.failure, stage: "storage_selection", message: "保存先選択の検証は要確認です", meetingRoot: null };
if (variant === "invalid") f.report = { schemaVersion: 1, phase: "ready", failure: report.failure };
if (variant === "long") f.report.failure = { ...report.failure, message: "長い起動診断を保持します。".repeat(250), messageTruncated: true };
mockIPC(async (command, args) => {
  if (command === "plugin:window|title") return document.title;
  if (command === "plugin:window|is_maximized") return false;
  f.calls.push({ command, args });
  if (command === "get_startup_reset_state") return { operationId: null, busy: false, completed: false, message: "", error: null };
  if (command === "retry_startup") { if (f.holdRetry) await new Promise(resolve => { f.pendingRetry = resolve; }); if (f.retryFail) throw Error("合成再起動失敗"); return null; }
  if (command === "get_startup_report") { if (f.delay) await new Promise(resolve => { f.pendingRead = resolve; }); if (f.readFail) throw Error("合成診断の読込失敗"); return structuredClone(f.report); }
  if (command === "quit_blocked_startup") { if (f.delay) await new Promise(resolve => { f.pendingQuit = resolve; }); if (f.quitFail) throw Error("合成終了の応答喪失"); return null; }
  throw Error(`Unexpected startup command: ${command}`);
}, { shouldMockEvents: true });
if (variant.startsWith("transition-")) {
  const [{ createElement }, { createRoot }, { StartupGate }] = await Promise.all([
    import("react"), import("react-dom/client"), import("../src/startup-attention.tsx"),
  ]);
  createRoot(document.getElementById("root")).render(createElement(StartupGate, null,
    createElement("main", { "aria-label": "通常画面" }, "通常画面")));
  setTimeout(() => { f.report = variant === "transition-ready"
    ? { schemaVersion: 1, phase: "ready", failure: null } : structuredClone(report); }, 600);
} else await import("../src/main.tsx");
