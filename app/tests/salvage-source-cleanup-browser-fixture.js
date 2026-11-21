// The settings and native bridge use the production frontend. All old-file
// receipts and deletion responses are synthetic and confined to this page.
import { mockIPC } from "./startup-ready-mocks.ts";

window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const data = window.dataFixture;
const operationId = "77777777-7777-4777-8777-777777777777";
const result = {
  operationId,
  sourceRoot: "/safe/元の会議データ",
  sourceModels: "/safe/元のモデル",
  destinationRoot: "/safe/復元後の会議データ",
  destinationModels: "/safe/復元後のモデル",
  meetingCount: 2,
  voicePersonCount: 1,
  modelCount: 1,
  items: [
    { kind: "minutes", sourceId: "sample-meeting", label: "設備点検の定例会議", status: "recoverable", reason: null },
    { kind: "transcript", sourceId: "sample-meeting", label: "設備点検の定例会議", status: "recoverable", reason: null },
    { kind: "audio", sourceId: "sample-meeting", label: "設備点検の定例会議", status: "unavailable", reason: "元の音声を確認できませんでした。" },
    { kind: "audio", sourceId: "deleted-audio-meeting", label: "運用手順の見直し", status: "deleted", reason: null },
  ],
};
const cleanup = window.salvageCleanupFixture = {
  status: { sourceRoot: result.sourceRoot, databaseFiles: 3, audioFiles: 12, state: "available" },
  calls: [],
  mode: "complete",
  finish: null,
  resultMode: "available",
  finishResult: null,
};

mockIPC(async (command, args) => {
  if (command === "get_asr_backends") return [];
  if (command === "get_storage_bootstrap") {
    const storage = await dataSaveIPC(command, args);
    return { ...storage, activeMeetingRoot: result.destinationRoot, meetingRoot: result.destinationRoot, modelsRoot: result.destinationModels,
      origin: "salvaged", salvageAvailable: true };
  }
  if (command === "get_salvage_result") {
    if (cleanup.resultMode === "hold") await new Promise(resolve => { cleanup.finishResult = resolve; });
    if (cleanup.resultMode === "empty") return null;
    if (cleanup.resultMode === "error") throw Error("復元結果を読み込めませんでした。");
    return structuredClone(result);
  }
  if (command === "get_salvage_source_cleanup") {
    if (args.operationId !== operationId) throw Error("復元操作が違います");
    return structuredClone(cleanup.status);
  }
  if (command === "delete_salvage_source") {
    if (args.operationId !== operationId) throw Error("削除対象が違います");
    cleanup.calls.push(structuredClone(args));
    if (cleanup.mode === "hold") await new Promise(resolve => { cleanup.finish = resolve; });
    if (cleanup.mode === "interrupt") {
      cleanup.status = { ...cleanup.status, state: "interrupted" };
      throw Error("削除を完了できませんでした。残っている対象を再確認してください。");
    }
    if (cleanup.mode === "unknown") throw Error("削除結果の受信に失敗しました。");
    cleanup.status = { ...cleanup.status, state: "completed" };
    if (cleanup.mode === "lostAfterSuccess") throw Error("完了応答が失われました。");
    return structuredClone(cleanup.status);
  }
  if (command === "get_data_operation_availability" && data.blocker) {
    return { available: false, reason: data.blocker, reasonCode: "operation_active", deletionAvailable: false, deletionReason: data.blocker, deletionReasonCode: "operation_active" };
  }
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });

await import("../src/bootstrap.tsx");
