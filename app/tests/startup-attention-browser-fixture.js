// Dedicated denied/unknown/ready startup scenarios. Unlike existing suite
// baselines, this uses the unwrapped native mock and production root gate.
import "../src/style.css";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
window.isTauri = true; mockWindows("main");
const variant = new URLSearchParams(location.search).get("fixture") ?? "blocked";
const report = { schemaVersion: 1, phase: "blocked", failure: { stage: "storage_admission", message: "一時領域台帳が不正です。<img src=x onerror=alert(1)>\n元の音声を保持します。", messageTruncated: false, meetingRoot: "/safe/会議保存先", configRoot: "/safe/起動設定" } };
const f = window.startupFixture = { calls: [], report, readFail: variant === "read-fail", quitFail: false, delay: false, pendingRead: null, pendingQuit: null };
f.activate = requestId => emit("startup-activate-requested", { requestId });
if (variant.startsWith("windows")) {
  f.report.platform = "windows";
  f.report.failure.message = "既存のMONから画面表示の応答がありません。ここから復旧できます。";
  f.instance = { schemaVersion: 1, supported: true, targetId: "11111111-1111-4111-8111-111111111111", canRestart: true, message: "既存のMONを確認して復旧できます。" };
  if (variant === "windows-no-owner") { f.instance.targetId = null; f.instance.canRestart = false; }
  if (variant === "windows-invalid") f.instance.targetId = "unverified-pid";
  f.opened = false; f.failRecover = false; f.holdRecover = false; f.pendingRecover = null;
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
  if (command === "get_startup_instance") return structuredClone(f.instance);
  if (command === "open_existing_instance") return f.opened;
  if (command === "recover_startup_instance") {
    if (f.holdRecover) await new Promise(resolve => { f.pendingRecover = resolve; });
    if (f.failRecover) throw Error("合成復旧応答喪失");
    if (args.targetId !== f.instance.targetId) throw Error("合成復旧対象変更");
    f.instance = { ...f.instance, targetId: null, canRestart: false }; return null;
  }
  if (command === "retry_startup" || command === "acknowledge_startup_activation") return null;
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
