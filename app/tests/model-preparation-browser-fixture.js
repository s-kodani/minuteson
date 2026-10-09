// Synthetic IPC only: no real model directory, process or user data is opened.
import { mockIPC, mockWindows } from "./startup-ready-mocks.ts";
import { emit } from "@tauri-apps/api/event";
import { dataIdleRead } from "./data-idle-browser-fixture.js";
const hash = "a".repeat(64), packageId = "11111111-1111-4111-8111-111111111111";
const entry = { packageId, modelName: "合成CPUモデル", modelVersion: "fixture", engine: "whisper.cpp", engineVersion: "declared-fixture", totalBytes: 1024, placementRecorded: true, ready: false, reviewHash: hash };
const f = window.modelPreparationFixture = { calls: [], mode: "success", failCatalog: false, failInspect: false, failCancel: false, cancelled: false, activeToken: null, complete: () => {}, entries: [entry], recording: { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null } };
f.recordingChanged = async () => { f.recording = { ...f.recording, revision: f.recording.revision + 1, phase: "recording", meetingId: "synthetic-meeting", sessionId: "synthetic-session" }; await emit("session-changed", structuredClone(f.recording)); };
window.isTauri = true; mockWindows("main");
mockIPC(async (command, args) => {
  const baseline = dataIdleRead(command); if (baseline !== undefined) return baseline;
  f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_recording_snapshot") return structuredClone(f.recording);
  if (command === "capture_available") return false;
  if (command === "list_meetings") return [];
  if (command === "get_vocabulary_book") return { id: "global", payload: [], lockVersion: 0 };
  if (command === "get_audio_import") return { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, saved: null, errorMessage: null };
  if (command === "get_model_placements") { if (f.failCatalog) throw Error("模擬配置記録の読込失敗"); return structuredClone(f.entries); }
  if (command === "get_model_profiles") return ["standard", "high_accuracy"].map(name => ({ name, id: null, profileVersion: 0, components: [], configurationComplete: false }));
  if (command === "inspect_model_package") { if (f.failInspect) throw Error("模擬事前検査失敗"); return { manifest: { modelName: "合成コピー素材", modelVersion: "fixture", engine: "whisper.cpp", engineVersion: "declared-fixture", target: "macos_aarch64", licenseRef: "fixture only", quantization: null }, manifestHash: hash, reviewHash: hash, totalBytes: 1024, engineCompatibility: "not_verified", publisherAuthenticity: "not_verified", installed: false, ready: false, files: [{ relativePath: "fixture-cli", role: "executable", actualSize: 512, actualHash: hash, expectedSizeChecked: true, expectedHashChecked: true }, { relativePath: "fixture-model", role: "asr", actualSize: 512, actualHash: hash, expectedSizeChecked: true, expectedHashChecked: true }] }; }
  if (command === "cancel_model_inspection") {
    if (args.token !== f.activeToken) throw Error("停止要求の操作ID不一致");
    if (f.failCancel) throw Error("模擬停止要求の応答喪失");
    f.cancelled = true; return null;
  }
  if (command === "place_model_package" || command === "check_model_compatibility") {
    if (args.expectedReviewHash !== hash) throw Error("確認した素材ハッシュ不一致");
    f.activeToken = args.token; f.cancelled = false;
    const finish = () => {
      if (command === "place_model_package") {
        if (f.cancelled) throw Error("模擬コピー中断・元素材保持");
        const copied = { ...entry, packageId: args.token, modelName: "合成コピー素材" }; f.entries.push(copied);
        if (f.mode === "lost") throw Error("模擬配置応答喪失・配置記録あり");
        return { packageId: args.token, reviewHash: hash, totalBytes: 1024, placementRecorded: true, engineCompatibility: "not_verified", ready: f.mode === "forged" };
      }
      if (args.packageId !== packageId) throw Error("確認対象のパッケージID不一致");
      const state = f.cancelled ? "cancelled" : f.mode === "failed" ? "failed" : "succeeded";
      const summary = { operationId: args.token, packageId, reviewHash: hash, adapterRevision: 1, state, errorMessage: state === "succeeded" ? null : "合成試行の終了" };
      entry.compatibility = structuredClone(summary); return summary;
    };
    if (f.mode === "delay") return new Promise((resolve, reject) => { f.complete = () => { try { resolve(finish()); } catch (cause) { reject(cause); } }; });
    return finish();
  }
  throw Error(`Unexpected preparation fixture command: ${command}`);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
