// Composition of existing synthetic IPC fixtures. Never downloads or runs AI.
import { mockIPC } from "./startup-ready-mocks.ts";
import { emit } from "@tauri-apps/api/event";
window.skipDataFixtureMain = true;
window.skipModelDownloadFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const { modelDownloadIPC } = await import("./model-download-browser-fixture.tsx");
const modelCommands = new Set(["get_model_license_review", "get_official_model_options", "get_model_placements", "get_model_profiles", "get_model_download_catalog", "start_model_download", "place_downloaded_model", "check_model_compatibility", "preview_model_profile_application", "apply_model_profile_application", "cancel_model_inspection", "get_model_download_lifetime", "read_model_download", "get_downloaded_model_placement_result"]);
const f = window.operationProgressFixture = {
  emitModelWait: (operationId, waiting) => emit("model-preparation-wait", { operationId, waiting }),
  emitTransfer: async (receivedBytes, patch = {}) => {
    const start = window.modelDownloadFixture.calls.find(call => call.command === "start_model_download");
    if (!start) throw Error("Download has not started");
    const operation = window.modelDownloadFixture.operations.get(start.args.token);
    await emit("model-download-progress", { operationId: start.args.token, recipeHash: start.args.expectedRecipeHash, totalBytes: operation.review.downloadBytes, receivedBytes, ...patch });
  },
};
mockIPC(async (command, args) => {
  if (modelCommands.has(command)) return modelDownloadIPC(command, args);
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
await import("../src/bootstrap.tsx");
