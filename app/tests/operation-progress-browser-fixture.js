// Composition of existing synthetic IPC fixtures. Never downloads or runs AI.
import { mockIPC } from "./startup-ready-mocks.ts";
import { emit } from "@tauri-apps/api/event";
window.skipDataFixtureMain = true;
window.skipModelDownloadFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const { modelDownloadIPC } = await import("./model-download-browser-fixture.tsx");
const modelCommands = new Set(["get_model_license_review", "get_official_model_options", "get_model_placements", "get_model_profiles", "get_model_download_catalog", "start_model_download", "place_downloaded_model", "check_model_compatibility", "preview_model_profile_application", "apply_model_profile_application", "cancel_model_inspection", "get_model_download_lifetime", "read_model_download", "get_downloaded_model_placement_result"]);
const cleanup = window.modelCleanupFixture = { snapshot: { revision: 0, active: null } };
const publishCleanup = async active => {
  cleanup.snapshot = { revision: cleanup.snapshot.revision + 1, active };
  await emit("model-cleanup-changed", cleanup.snapshot);
};
const f = window.operationProgressFixture = {
  closeCalls: [], hidden: false, hideFailure: false,
  holdCleanup: false,
  finishCleanup: async () => {
    const job = cleanup.snapshot.active;
    cleanup.history = { items: [{ id: `model:${job.id}`, kind: "model", target: job.target, meetingId: null,
      state: "attention", acceptedAt: job.startedAtMs, finishedAt: null, observedAt: Date.now(),
      detail: "モデル設定は反映済み。重複モデルの整理は未完了" }], nextCursor: null, storageError: null };
    await publishCleanup(null);
    await emit("job-history-changed", null);
    f.releaseCleanup();
  },
  requestClose: quit => emit("main-close-requested", { quit }),
  emitModelWait: (operationId, waiting) => emit("model-preparation-wait", { operationId, waiting }),
  emitTransfer: async (receivedBytes, patch = {}) => {
    const start = window.modelDownloadFixture.calls.find(call => call.command === "start_model_download");
    if (!start) throw Error("Download has not started");
    const operation = window.modelDownloadFixture.operations.get(start.args.token);
    await emit("model-download-progress", { operationId: start.args.token, recipeHash: start.args.expectedRecipeHash, totalBytes: operation.review.downloadBytes, receivedBytes, ...patch });
  },
};
mockIPC(async (command, args) => {
  if (command === "get_application_close_activity") return { analysisBusy: false, modelPreparationBusy: window.modelDownloadFixture.active.size > 0 };
  if (command === "get_residency_state") return { supported: true, canBackground: true, trayAvailable: true, dockAvailable: false, error: null };
  if (command === "hide_main_to_background") {
    f.closeCalls.push({ command, args });
    if (f.hideFailure) throw Error("模擬ウィンドウ非表示失敗");
    f.hidden = true;
    return null;
  }
  if (command === "apply_model_profile_application" && f.holdCleanup) {
    const result = await modelDownloadIPC(command, args);
    const pending = new Promise(resolve => { f.releaseCleanup = resolve; });
    await publishCleanup({ id: args.historyOperationId, target: "Synthetic official model",
      appliedPackage: args.preview.packageId, startedAtMs: Date.now() });
    await pending;
    return result;
  }
  if (modelCommands.has(command)) return modelDownloadIPC(command, args);
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
await import("../src/bootstrap.tsx");
