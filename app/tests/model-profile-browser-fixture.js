// Development-only mocked IPC. Not imported by the application or its build.
import { mockIPC, mockWindows } from "./startup-ready-mocks.ts";
import { dataIdleRead } from "./data-idle-browser-fixture.js";
const engines = ["whisper.cpp", "sherpa-onnx", "llama.cpp"];
const role = { "whisper.cpp": ["asr"], "sherpa-onnx": ["segmentation", "speaker_embedding"], "llama.cpp": ["llm"] };
const ids = engines.map((_, i) => `${i + 1}1111111-1111-4111-8111-111111111111`);
const checks = engines.map((_, i) => `${i + 1}2222222-2222-4222-8222-222222222222`);
const defaults = {
  "whisper.cpp": { executablePath: "", modelPath: "", vadModelPath: null, threads: 7, useGpu: true },
  "sherpa-onnx": { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [], threads: 6, clusterThresholdMilli: 700, windowShiftMilli: 400, minDurationOnMs: 200, minDurationOffMs: 600 },
  "llama.cpp": { executablePath: "", modelPath: "", threads: 5, contextTokens: 8192, predictionTokens: 512, gpuLayers: 2, seed: 777, promptBudget: 5000 },
};
const stored = JSON.parse(sessionStorage.getItem("mon-model-profile-fixture") || "null");
const data = window.modelProfileFixture = {
  calls: [], mode: "success", previewMode: "success", pending: null, complete: () => {},
  profiles: stored?.profiles ?? ["standard", "high_accuracy"].map(name => ({ name, id: null, profileVersion: 0, components: [], configurationComplete: false })),
  preferences: stored?.preferences ?? Object.fromEntries(engines.map(engine => [engine, { lockVersion: 1, payload: { standard: structuredClone(defaults[engine]), highAccuracy: structuredClone(defaults[engine]) } }])),
  failCatalog: false, failReadback: false,
};
const persist = () => sessionStorage.setItem("mon-model-profile-fixture", JSON.stringify({ profiles: data.profiles, preferences: data.preferences }));
const entries = engines.map((engine, i) => ({ packageId: ids[i], modelName: `${engine} 検証モデル`, modelVersion: "fixture-1", engine, engineVersion: "declared only", totalBytes: 1024, placementRecorded: true, ready: false, reviewHash: "a".repeat(64), compatibility: { operationId: checks[i], packageId: ids[i], reviewHash: "a".repeat(64), adapterRevision: 1, state: "succeeded", errorMessage: null } }));
entries.push({ ...structuredClone(entries[0]), packageId: "44444444-4444-4444-8444-444444444444", modelName: "失敗試行のモデル", compatibility: { ...structuredClone(entries[0].compatibility), packageId: "44444444-4444-4444-8444-444444444444", state: "failed", errorMessage: "fixture failure" } });
const configured = (engine, before, packageId) => {
  const directory = `/app/models/${packageId}/`;
  const after = { ...structuredClone(before), executablePath: directory + "cli" };
  if (engine === "sherpa-onnx") Object.assign(after, { segmentationModelPath: directory + "seg.onnx", embeddingModelPath: directory + "emb.onnx", runtimeLibraryPaths: [directory + "lib.dylib"] });
  else after.modelPath = directory + "model.bin";
  return after;
};
window.isTauri = true;
mockWindows("main");
mockIPC(async (command, args) => {
  const baseline = dataIdleRead(command);
  if (baseline !== undefined) return baseline;
  data.calls.push({ command, args: structuredClone(args) });
  if (command === "get_recording_snapshot") return { revision: 1, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null };
  if (command === "capture_available") return false;
  if (command === "list_meetings") return [];
  if (command === "get_vocabulary_book") return { id: "global", payload: [], lockVersion: 0 };
  if (command === "get_audio_import") return { revision: 1, phase: "idle", busy: false, recordingBlocked: false, token: null, fileName: null, info: null, sourceBytes: null, requiredBytes: null, availableBytes: null, completed: 0, saved: null, errorMessage: null };
  if (command === "get_model_placements") return structuredClone(entries);
  if (command === "get_model_profiles") { if (data.failCatalog) throw Error("模擬構成再読込失敗"); return structuredClone(data.profiles); }
  if (command === "cancel_model_inspection") return null;
  const preferenceEngine = { get_analysis_preferences: "whisper.cpp", get_diarization_preferences: "sherpa-onnx", get_minutes_preferences: "llama.cpp" }[command];
  if (preferenceEngine) { if (data.failReadback) throw Error("模擬設定再読込失敗"); return { id: "fixture", ...structuredClone(data.preferences[preferenceEngine]) }; }
  if (command === "preview_model_profile_application") {
    const entry = entries.find(e => e.packageId === args.packageId), profile = data.profiles.find(p => p.name === args.profile);
    const preferences = data.preferences[entry.engine], key = args.profile === "standard" ? "standard" : "highAccuracy";
    const result = { schemaVersion: 1, profile: args.profile, packageId: entry.packageId, reviewHash: entry.reviewHash, checkOperationId: entry.compatibility.operationId, expectedProfileId: profile.id, expectedProfileHash: profile.id ? "c".repeat(64) : null, expectedProfileVersion: profile.profileVersion, expectedPreferencesVersion: preferences.lockVersion, before: { engine: entry.engine, settings: structuredClone(preferences.payload[key]) }, after: { engine: entry.engine, settings: configured(entry.engine, preferences.payload[key], entry.packageId) }, reviewDigest: "b".repeat(64) };
    if (data.previewMode === "forged") result.after.settings.threads = 1;
    if (data.previewMode === "delay") return new Promise(resolve => { data.pending = args.token; data.complete = () => { data.pending = null; resolve(result); }; });
    return result;
  }
  if (command === "apply_model_profile_application") {
    const p = args.preview, entry = entries.find(e => e.packageId === p.packageId), profile = data.profiles.find(v => v.name === p.profile);
    if (profile.id === args.token) return structuredClone(profile);
    if (data.mode === "conflict" || profile.id !== p.expectedProfileId || profile.profileVersion !== p.expectedProfileVersion || data.preferences[entry.engine].lockVersion !== p.expectedPreferencesVersion) throw Error("模擬設定競合。再確認してください");
    const commit = () => {
      const key = p.profile === "standard" ? "standard" : "highAccuracy";
      const prefs = data.preferences[entry.engine];
      if (JSON.stringify(prefs.payload[key]) !== JSON.stringify(p.after.settings)) prefs.lockVersion++;
      prefs.payload[key] = structuredClone(p.after.settings);
      profile.id = args.token; profile.profileVersion++;
      profile.components = [...profile.components.filter(c => c.engine !== entry.engine), { roles: role[entry.engine], packageId: entry.packageId, modelName: entry.modelName, modelVersion: entry.modelVersion, engine: entry.engine, checkOperationId: entry.compatibility.operationId, settingsMatch: true, compatibilityCurrent: true }];
      profile.configurationComplete = profile.components.length === 3;
      persist(); return structuredClone(profile);
    };
    if (data.mode === "delay" || data.mode === "cancelled") return new Promise((resolve, reject) => { data.pending = args.token; data.complete = () => { data.pending = null; if (data.mode === "cancelled") reject(Error("模擬取消・書込なし")); else resolve(commit()); }; });
    const result = commit();
    if (data.mode === "lost") throw Error("模擬応答喪失（保存は確定済み）");
    return result;
  }
  throw Error(`Unexpected model profile fixture command: ${command}`);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
