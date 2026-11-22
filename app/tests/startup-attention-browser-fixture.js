// Dedicated denied/unknown/ready startup scenarios. Unlike existing suite
// baselines, this uses the unwrapped native mock and production root gate.
import "../src/style.css";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
window.isTauri = true; mockWindows(new URLSearchParams(location.search).get("window")??"main");
const variant = new URLSearchParams(location.search).get("fixture") ?? "blocked";
const productName = new URLSearchParams(location.search).get("productName") ?? "MinutesOn";
const report = { schemaVersion: 1, phase: "blocked", failure: { stage: "storage_admission", message: "会議データベースが破損しています。<img src=x onerror=alert(1)>\n元の音声を保持します。", messageTruncated: false, meetingRoot: "/safe/会議保存先", configRoot: "/safe/起動設定", salvageAvailable: true } };
const f = window.startupFixture = { calls: [], report, readFail: variant === "read-fail", quitFail: false, delay: false, pendingRead: null, pendingQuit: null };
let transientFailures = variant === "transient-read-fail" ? 2 : 0;
const salvageId = "12345678-1234-4123-8123-123456789012";
const secondSalvageId = "87654321-4321-4321-8321-210987654321";
if (variant === "partial-salvage") f.report.failure.stage = "partial_salvage";
f.salvage = { phase: "idle", error: null, selection: { operationId: salvageId, sourceRoot: "/safe/元の会議データ", destinationRoot: null, availableBytes: null, sourceSelectable: false, preview: null, result: null } };
f.resetState = { operationId: null, busy: false, completed: false, message: "", error: null };
if (variant === "reset-running") f.resetState = { operationId: salvageId, busy: true, completed: false, message: "旧データを退避しています", error: null };
if (variant === "reset-stopped-empty") f.resetState = { operationId: salvageId, busy: false, completed: false, message: "", error: null };
f.emitResidency = (event,payload) => emit(event,payload);
if (variant.startsWith("windows")) {
  f.report.platform = "windows";
  if (variant === "windows") {
    f.report.failure.message = "ほかのMinutesOnが同じ保存先を使用しています。使用中のMinutesOnを終了してください。";
    f.report.failure.salvageAvailable = false;
  }
}
if (variant === "legacy-salvage-field") delete f.report.failure.salvageAvailable;
if (variant === "malformed-salvage-field") f.report.failure.salvageAvailable = "true";
if (variant === "blocked-clean") f.report.failure.message = "会議データベースの破損を確認しました。\n保存先: /safe/会議アーカイブ\n元の会議データと音声ファイルを保持しています。";
if (variant === "ready") f.report = { schemaVersion: 1, phase: "ready", failure: null };
if (variant === "starting" || variant === "windows-starting" || variant.startsWith("transition-")) f.report = { schemaVersion: 1, phase: "starting", failure: null,
  ...(variant === "windows-starting" ? { platform: "windows" } : {}) };
if (variant === "selection") f.report.failure = { ...report.failure, stage: "storage_selection", message: "起動設定にアクセスできませんでした。読み取り権限を確認してください。", meetingRoot: null, salvageAvailable: false };
if (variant === "replaced-registration") f.report.failure = { ...report.failure, stage: "storage_selection", message: "登録したフォルダーまたは会議データのファイルが現在のものと一致しません。保存先を確認してください。", salvageAvailable: false };
if (variant === "invalid") f.report = { schemaVersion: 1, phase: "ready", failure: report.failure };
if (variant === "long") f.report.failure = { ...report.failure, message: "長い起動診断を保持します。".repeat(250), messageTruncated: true };
mockIPC(async (command, args) => {
  if (command === "get_window_environment") return {schemaVersion:1,productName,role:new URLSearchParams(location.search).get("window")??"main",platform:f.report.platform??"macos",capabilities:{voiceIdentity:true}};
  if (command === "plugin:window|title") return document.title;
  if (command === "plugin:window|is_maximized") return false;
  f.calls.push({ command, args });
  if (variant === "ready-salvage-edit") {
    if (command === "get_storage_write_state") return { readOnly: true, reasons: [], salvageAvailable: true };
    if (command === "get_audio_import" || command === "get_data_operation") return { token: null, busy: false };
    if (command === "export_salvage_edits") {
      if (f.exportFails) throw Error("保存先にファイルを書き込めませんでした。");
      return f.exportSaved === true;
    }
    if (command === "finish_main_close") return null;
  }
  if (command === "recheck_salvage_environment") return ["保存先が接続されています。アクセス権限と空き容量を確認しました。"];
  if (["open_salvage", "get_salvage_state", "choose_salvage_folder", "inspect_salvage", "commit_salvage", "leave_salvage"].includes(command)) {
    if (command === "leave_salvage") return null;
    const view = f.salvage.selection;
    if ((command === "inspect_salvage" || command === "commit_salvage") && f.holdWork) {
      f.salvage.phase = command === "inspect_salvage" ? "inspecting" : "building";
      await new Promise(resolve => { f.finishWork = resolve; });
      f.salvage.phase = "idle";
      f.salvage.progress = null;
    }
    if (command === "choose_salvage_folder" && !f.folderCancelled) {
      if (view.destinationRoot) {
        view.operationId = secondSalvageId;
        view.destinationRoot = "/safe/外付けドライブ/新しい保存先";
        view.availableBytes = 3 * 1024 * 1024;
        view.preview = null;
      } else {
        view.destinationRoot = "/safe/新しい保存先";
        view.availableBytes = 256;
      }
    }
    if (command === "inspect_salvage") {
      if (f.capacityFreed) view.availableBytes = 3 * 1024 * 1024;
      view.preview = { operationId: view.operationId, sourceRoot: view.sourceRoot, requiredBytes: 2 * 1024 * 1024, meetingTotal: null, recoverableCount: 1, items: [
        { kind: "minutes", sourceId: "readable-meeting", label: "確定議事録", status: "recoverable", reason: null },
        { kind: "audio", sourceId: null, label: "残りの会議の音声", status: "uninspected", reason: "会議データを読み取れず、残りの音声を確認できませんでした" },
      ] };
    }
    return structuredClone(f.salvage);
  }
  if (command === "get_startup_reset_state") return structuredClone(f.resetState);
  if (command === "get_startup_reset_preview") {
    if (args?.restore !== undefined) throw Error("初期化の確認に復元引数を送信しました");
    return { availability: { available: true, reason: null }, operationId: "12345678-1234-4123-8123-123456789012", meetingRoot: "/safe/会議保存先", archivePath: null };
  }
  if (command === "choose_startup_reset_backup_directory") return "/safe/初期化前の退避";
  if (command === "start_startup_reset") {
    f.resetState = { operationId: args.operationId, busy: true, completed: false, message: "旧データを退避しています", error: null };
    if (f.startReplyFail) throw Error("合成受付応答の喪失");
    return structuredClone(f.resetState);
  }
  if (command === "retry_startup") { if (f.holdRetry) await new Promise(resolve => { f.pendingRetry = resolve; }); if (f.retryFail) throw Error("合成再起動失敗"); return null; }
  if (command === "get_startup_report") { if (f.delay) await new Promise(resolve => { f.pendingRead = resolve; }); if (f.readFail || transientFailures-- > 0) throw Error("起動診断の読み込みに失敗しました。"); return structuredClone(f.report); }
  if (command === "quit_blocked_startup") { if (f.delay) await new Promise(resolve => { f.pendingQuit = resolve; }); if (f.quitFail) throw Error("合成終了の応答喪失"); return null; }
  throw Error(`Unexpected startup command: ${command}`);
}, { shouldMockEvents: true });
if (variant === "ready-salvage-edit") {
  const [{ createElement, Fragment }, { createRoot }, { SalvageEntry }, { registerPendingEdits }, { ToastHost }] = await Promise.all([
    import("react"), import("react-dom/client"), import("../src/salvage.tsx"), import("../src/draft-save.ts"), import("../src/toast.tsx"),
  ]);
  f.liveEdit = "編集中の議事録";
  registerPendingEdits(async () => { throw Error("保存先は閲覧専用です。"); }, () => true,
    () => ({ kind: "minutes", meetingId: "meeting", input: { markdown: f.liveEdit } }));
  createRoot(document.getElementById("root")).render(createElement(Fragment, null,
    createElement(SalvageEntry, { mode: "ready" }), createElement(ToastHost)));
} else if (variant.startsWith("transition-")) {
  const [{ createElement }, { createRoot }, { StartupGate }] = await Promise.all([
    import("react"), import("react-dom/client"), import("../src/startup-attention.tsx"),
  ]);
  createRoot(document.getElementById("root")).render(createElement(StartupGate, null,
    createElement("main", { "aria-label": "通常画面" }, "通常画面")));
  setTimeout(() => { f.report = variant === "transition-ready"
    ? { schemaVersion: 1, phase: "ready", failure: null } : structuredClone(report); }, 600);
} else await import("../src/bootstrap.tsx");
