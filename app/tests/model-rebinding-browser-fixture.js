// Development-only mocked IPC; never imported by the application build.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { rebindingFixture, meetingId } from "./model-rebinding-fixture.ts";
const fixtures = ["transcription", "diarization", "minutes_generation"].map(rebindingFixture);
const meetings = [meetingId, "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"].map((id, i) => ({ id, title: `再配置確認会議${i + 1}`, lifecycle: "recorded", lockVersion: 1, createdAt: 1791160000000, updatedAt: 1791160000000 }));
const initial = () => fixtures.map(f => structuredClone(f.job));
const data = window.rebindingFixture = { calls: [], jobs: initial(), mode: "success", previewMode: "success", pending: null, complete: () => {}, cancelled: new Set(), receipts: new Map(), failCatalog: false, failReceipt: false, failSave: false };
const preferences = {
  get_analysis_preferences: { executablePath: "", modelPath: "", vadModelPath: null, threads: 4, useGpu: false },
  get_diarization_preferences: { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [], threads: 4, clusterThresholdMilli: 500, windowShiftMilli: 500, minDurationOnMs: 300, minDurationOffMs: 500 },
  get_minutes_preferences: { executablePath: "", modelPath: "", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 },
};
window.isTauri = true; mockWindows("main");
mockIPC(async (command, args) => {
  data.calls.push({ command, args: structuredClone(args) });
  if (command === "save_analysis_preferences") { if (data.failSave) throw Error("模擬設定保存競合"); return { id: "fixture", lockVersion: args.expectedLockVersion + 1, payload: structuredClone(args.payload) }; }
  if (preferences[command]) return { id: "fixture", lockVersion: 0, payload: { standard: structuredClone(preferences[command]), highAccuracy: structuredClone(preferences[command]) } };
  if (command === "list_meetings") return structuredClone(meetings);
  if (command === "get_meeting") return structuredClone(meetings.find(m => m.id === args.id));
  if (command === "capture_available") return false;
  if (command === "get_recording_snapshot") return { revision: 1, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null };
  if (command === "get_audio_import") return { revision: 1, phase: "idle", busy: false, recordingBlocked: false, token: null, fileName: null, info: null, sourceBytes: null, requiredBytes: null, availableBytes: null, completed: 0, saved: null, errorMessage: null };
  if (["get_current_analysis", "get_transcript_draft", "get_analysis_flow", "get_current_minutes", "get_minutes_draft"].includes(command)) return null;
  if (["get_analysis_versions", "get_minutes_versions"].includes(command)) return { meetingLockVersion: 1, currentId: null, previousId: null, draft: null, versions: [] };
  if (command === "get_analysis_jobs") return args.meetingId === meetingId ? structuredClone(data.jobs) : [];
  if (command === "get_minutes_exports") return [];
  if (command === "get_diarization_tracks") return { tracks: [] };
  if (command === "get_vocabulary_book") return { id: "global", payload: [], lockVersion: 0 };
  if (command === "get_meeting_vocabulary") return { global: { id: "global", payload: [], lockVersion: 0 }, meeting: { id: "meeting", payload: [], lockVersion: 0 } };
  if (command === "get_model_profiles") return ["standard", "high_accuracy"].map(name => ({ name, id: null, profileVersion: 0, components: [], configurationComplete: false }));
  if (command === "get_model_placements") { if (data.failCatalog) throw Error("模擬配置読込失敗"); return fixtures.map(f => structuredClone(f.entry)); }
  if (command === "cancel_model_inspection") { data.cancelled.add(args.token); return null; }
  if (command === "preview_job_model_rebinding") {
    const f = fixtures.find(f => f.job.id === args.jobId);
    if (!f || args.meetingId !== meetingId || args.expectedSpecHash !== f.job.specHash || args.packageId !== f.entry.packageId || args.expectedReviewHash !== f.entry.reviewHash || args.checkOperationId !== f.entry.compatibility.operationId) throw Error("模擬確認条件不一致");
    const result = structuredClone(f.preview);
    if (data.previewMode === "forged") result.binding.files[0].sha256 = "0".repeat(64);
    if (data.previewMode === "delay") return new Promise(resolve => { data.pending = args.token; data.complete = () => { data.pending = null; resolve(result); }; });
    return result;
  }
  if (command === "get_job_model_rebinding_result") {
    if (data.failReceipt) throw Error("模擬結果確認失敗");
    return { job: structuredClone(data.receipts.get(args.operationId) ?? null), active: data.pending === args.operationId, cancelled: data.cancelled.has(args.operationId) };
  }
  if (command === "apply_job_model_rebinding") {
    if (data.receipts.has(args.token)) return structuredClone(data.receipts.get(args.token));
    if (data.cancelled.has(args.token)) throw Error("模擬取消済み");
    const f = fixtures.find(f => f.job.id === args.preview.source.id);
    const commit = () => { const result = { ...structuredClone(f.result), operationId: args.token }; data.receipts.set(args.token, result); data.jobs.unshift(result); return structuredClone(result); };
    if (data.mode === "lostActive") { data.pending = args.token; data.complete = () => { data.pending = null; commit(); }; throw Error("模擬応答喪失・処理は継続中"); }
    if (data.mode === "conflict") throw Error("模擬条件競合・受付なし");
    if (["delay", "cancelled"].includes(data.mode)) return new Promise((resolve, reject) => { data.pending = args.token; data.complete = () => { data.pending = null; if (data.mode === "cancelled") reject(Error("模擬取消・受付なし")); else resolve(commit()); }; });
    const result = commit();
    if (data.mode === "lost") throw Error("模擬応答喪失・受付済み");
    if (data.mode === "forged") return { ...result, operationId: f.job.operationId };
    return result;
  }
  if (command === "finish_main_close") return null;
  throw Error(`Unexpected rebinding fixture command: ${command}`);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
