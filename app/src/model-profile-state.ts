import { checkedVadOptions, sameVadOptions, type VadOptions } from "./vad-state.ts";
import type { EnginePreferences } from "./analysis-state.ts";
import type { DiarizationEngine } from "./diarization-state.ts";
import type { MinutesEngine } from "./minutes-state.ts";
import { modelBodyAvailable, placementLabel, type ModelPlacementEntry } from "./model-inspection-state.ts";

export type ModelProfileName = "standard" | "high_accuracy";
export type ModelSettings = { engine: "whisper.cpp" | "asr"; settings: EnginePreferences }
  | { engine: "sherpa-onnx"; settings: DiarizationEngine }
  | { engine: "llama.cpp"; settings: MinutesEngine };
export interface ModelProfileComponent {
  roles: string[]; packageId: string; modelName: string; modelVersion: string;
  engine: ModelSettings["engine"]; checkOperationId: string; settingsMatch: boolean; compatibilityCurrent: boolean;
}
export interface ModelProfileView {
  name: ModelProfileName; id: string | null; profileVersion: number;
  components: ModelProfileComponent[]; configurationComplete: boolean;
}
export interface ModelApplicationPreview {
  schemaVersion: 1; profile: ModelProfileName; packageId: string; reviewHash: string; checkOperationId: string;
  expectedProfileId: string | null; expectedProfileHash: string | null;
  expectedProfileVersion: number; expectedPreferencesVersion: number;
  before: ModelSettings; after: ModelSettings; reviewDigest: string;
  whisperVadOptions?: VadOptions;
  diarizationThreads?: number;
  diarizationClusterThresholdMilli?: number;
}
const sameOptionalVad = (a: unknown, b: unknown) => a === undefined || b === undefined
  ? a === b : sameVadOptions(checkedVadOptions(a), checkedVadOptions(b));
const id = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(v);
const sha = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{64}$/.test(v);
const version = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const names = ["standard", "high_accuracy"];
const roles = { "asr": ["asr"], "whisper.cpp": ["asr"], "sherpa-onnx": ["segmentation", "speaker_embedding"], "llama.cpp": ["llm"] };
const fileKeys = ["executablePath", "modelPath", "vadModelPath", "segmentationModelPath", "embeddingModelPath", "runtimeLibraryPaths"];
const keys = {
  "asr": ["executablePath", "modelPath", "vadModelPath", "threads", "useGpu"],
  "whisper.cpp": ["executablePath", "modelPath", "vadModelPath", "threads", "useGpu"],
  "sherpa-onnx": ["executablePath", "segmentationModelPath", "embeddingModelPath", "runtimeLibraryPaths", "threads", "clusterThresholdMilli", "windowShiftMilli", "minDurationOnMs", "minDurationOffMs"],
  "llama.cpp": ["executablePath", "modelPath", "threads", "contextTokens", "predictionTokens", "gpuLayers", "seed", "promptBudget"],
};
function fail(): never { throw Error("モデル構成と確認内容が一致しません。記録を再読込し、適用内容を再確認してください。"); }
export function profileName(name: ModelProfileName) { return name === "standard" ? "標準" : "高精度"; }
export function canReviewApplication(entry: ModelPlacementEntry): boolean {
  placementLabel(entry);
  return entry.placementRecorded && modelBodyAvailable(entry) && entry.compatibility?.state === "succeeded";
}
export function assertProfileView(view: ModelProfileView) {
  if (!view || !names.includes(view.name) || !version(view.profileVersion) || !Array.isArray(view.components)
    || typeof view.configurationComplete !== "boolean" || view.components.length > 3
    || (view.id === null ? view.profileVersion !== 0 || view.components.length !== 0 : !id(view.id) || view.profileVersion < 1 || !view.components.length)) fail();
  const engines = new Set<string>();
  for (const component of view.components) {
    if (!component || !Object.hasOwn(roles, component.engine) || engines.has(component.roles?.[0])
      || !id(component.packageId) || !id(component.checkOperationId)
      || typeof component.modelName !== "string" || !component.modelName || typeof component.modelVersion !== "string" || !component.modelVersion
      || typeof component.settingsMatch !== "boolean" || typeof component.compatibilityCurrent !== "boolean"
      || JSON.stringify(component.roles) !== JSON.stringify(roles[component.engine])) fail();
    engines.add(component.roles[0]);
  }
  if (view.configurationComplete !== (engines.size === 3 && view.components.every(c => c.settingsMatch && c.compatibilityCurrent))) fail();
}
export function assertProfileCatalog(views: ModelProfileView[]) {
  if (!Array.isArray(views) || views.length !== 2 || new Set(views.map(v => v?.name)).size !== 2) fail();
  views.forEach(assertProfileView);
}
export function profileConfigurationLabel(view: ModelProfileView) {
  assertProfileView(view);
  if (!view.id) return "管理モデルは未選択";
  if (view.components.some(c => !c.settingsMatch || !c.compatibilityCurrent)) return "保存構成と現在の設定・互換性記録に不一致あり";
  return view.configurationComplete ? "3エンジンの保存構成が一致（実行準備・品質は別途確認）" : "一部の管理モデルを選択済み";
}
function settingsEntries(value: ModelSettings): [string, unknown][] {
  if (!value || !Object.hasOwn(keys, value.engine) || !value.settings || typeof value.settings !== "object") fail();
  const entries = Object.entries(value.settings);
  const vad = "vadOptions" in value.settings ? value.settings.vadOptions : undefined;
  if (vad !== undefined) checkedVadOptions(vad);
  if (entries.length !== keys[value.engine].length + Number(vad !== undefined) + Number("backend" in value.settings) || keys[value.engine].some(k => !Object.hasOwn(value.settings, k))) fail();
  if (entries.some(([k, v]) => (k === "backend" || k === "vadOptions") && ["whisper.cpp", "asr"].includes(value.engine) ? false : fileKeys.includes(k)
    ? (k === "runtimeLibraryPaths" ? !Array.isArray(v) || v.some(p => typeof p !== "string") : k === "vadModelPath" && v === null ? false : typeof v !== "string")
    : k === "useGpu" ? typeof v !== "boolean" : typeof v !== "number" || !Number.isSafeInteger(v) || v < 0)) fail();
  if ("backend" in value.settings && value.settings.backend !== undefined) {
    const b = value.settings.backend;
    if (!b || typeof b.id !== "string" || !b.id || typeof b.files !== "object" || !b.files || Array.isArray(b.files)
      || Object.values(b.files).some(path => typeof path !== "string" || !path || path.length > 4096 || path.includes("\0"))) fail();
  }
  if (value.settings.threads < 1 || value.settings.threads > 64) fail();
  return entries;
}
export function applicationChanges(preview: ModelApplicationPreview) {
  const before = new Map(settingsEntries(preview.before));
  return settingsEntries(preview.after).map(([key, after]) => ({ key, before: before.get(key), after, file: fileKeys.includes(key) }));
}
export function assertApplicationPreview(preview: ModelApplicationPreview, entry: ModelPlacementEntry, name: ModelProfileName, current: ModelProfileView, diarizationThreads?: number, diarizationClusterThresholdMilli?: number, whisperVadOptions?: VadOptions) {
  if (!canReviewApplication(entry) || !preview || preview.schemaVersion !== 1 || preview.profile !== name || current.name !== name
    || preview.packageId !== entry.packageId || preview.reviewHash !== entry.reviewHash || preview.checkOperationId !== entry.compatibility?.operationId
    || !sha(preview.reviewDigest) || !version(preview.expectedPreferencesVersion) || !version(preview.expectedProfileVersion)
    || preview.expectedProfileId !== current.id || preview.expectedProfileVersion !== current.profileVersion
    || (current.id === null ? preview.expectedProfileHash !== null : !sha(preview.expectedProfileHash))
    || preview.before.engine !== entry.engine || preview.after.engine !== entry.engine
    || !sameOptionalVad(preview.whisperVadOptions, whisperVadOptions)
    || whisperVadOptions !== undefined && (!["whisper.cpp", "asr"].includes(preview.after.engine) || !("vadOptions" in preview.after.settings) || !preview.after.settings.vadOptions || !sameVadOptions(checkedVadOptions(whisperVadOptions), checkedVadOptions(preview.after.settings.vadOptions)))
    || preview.diarizationThreads !== diarizationThreads
    || preview.diarizationClusterThresholdMilli !== diarizationClusterThresholdMilli
    || diarizationClusterThresholdMilli !== undefined && (preview.after.engine !== "sherpa-onnx" || !Number.isInteger(diarizationClusterThresholdMilli) || diarizationClusterThresholdMilli < 0 || diarizationClusterThresholdMilli > 2000 || preview.after.settings.clusterThresholdMilli !== diarizationClusterThresholdMilli)
    || diarizationThreads !== undefined && (entry.engine !== "sherpa-onnx" || !Number.isInteger(diarizationThreads) || diarizationThreads < 1 || diarizationThreads > 64 || preview.after.settings.threads !== diarizationThreads)) fail();
  assertProfileView(current);
  const tuning = diarizationThreads !== undefined || diarizationClusterThresholdMilli !== undefined;
  if (tuning && !current.components.some(component => component.engine === "sherpa-onnx" && component.packageId === entry.packageId && component.checkOperationId === preview.checkOperationId && component.settingsMatch && component.compatibilityCurrent)) fail();
  const backendChanged = ["whisper.cpp", "asr"].includes(preview.after.engine)
    && ((preview.before.settings as EnginePreferences).backend?.id ?? "whisper.cpp") !== ((preview.after.settings as EnginePreferences).backend?.id ?? "whisper.cpp");
  if ((preview.before.engine === "whisper.cpp" || preview.before.engine === "asr") && !backendChanged
    && preview.before.settings.vadOptions !== undefined && whisperVadOptions === undefined
    && !sameOptionalVad(preview.before.settings.vadOptions, (preview.after.settings as EnginePreferences).vadOptions)) fail();
  const hasVad = ["whisper.cpp", "asr"].includes(entry.engine) && entry.modelFiles?.some(f => f.role === "vad");
  for (const change of applicationChanges(preview)) {
    if (change.key === "backend") continue;
    if (change.key === "vadOptions" && backendChanged) {
      checkedVadOptions(change.after); continue;
    }
    if (change.key === "useGpu" && preview.after.engine === "asr" && change.after === false) continue;
    if (!change.file && JSON.stringify(change.before) !== JSON.stringify(change.after) && !(change.key === "vadOptions" && hasVad && (whisperVadOptions !== undefined || change.before === undefined && sameVadOptions(checkedVadOptions(change.after), { enabled: true, thresholdMilli: 600, minSpeechDurationMs: 300, minSilenceDurationMs: 100, maxSpeechDurationS: 30, speechPadMs: 30, samplesOverlapMs: 100 }))) && !(change.key === "threads" && diarizationThreads !== undefined) && !(change.key === "clusterThresholdMilli" && diarizationClusterThresholdMilli !== undefined)) fail();
    if (tuning && change.file && JSON.stringify(change.before) !== JSON.stringify(change.after)) fail();
    if (change.file) for (const path of Array.isArray(change.after) ? change.after : change.after === null ? [] : [change.after]) {
      if (typeof path !== "string" || path.length > 4096 || path.includes("\0") || !(/^\//.test(path) || /^[a-z]:[\\/]/i.test(path) || /^\\\\/.test(path))) fail();
    }
  }
}
export function assertApplicationResult(view: ModelProfileView, operationId: string, preview: ModelApplicationPreview) {
  assertProfileView(view);
  const component = view.components.find(c => c.engine === preview.after.engine);
  if (view.id !== operationId || view.name !== preview.profile || view.profileVersion !== preview.expectedProfileVersion + 1
    || !component || component.packageId !== preview.packageId || component.checkOperationId !== preview.checkOperationId
    || !component.settingsMatch || !component.compatibilityCurrent) fail();
}
export const preferenceCommand = (engine: ModelSettings["engine"]) => ({ "asr": "get_asr_preferences", "whisper.cpp": "get_asr_preferences", "sherpa-onnx": "get_diarization_preferences", "llama.cpp": "get_minutes_preferences" })[engine];
export function assertAppliedPreferences(value: { lockVersion: number; payload: { standard: unknown; highAccuracy: unknown } }, preview: ModelApplicationPreview) {
  const key = preview.profile === "standard" ? "standard" : "highAccuracy";
  const expected = Object.fromEntries(settingsEntries(preview.after));
  const actual = value?.payload?.[key];
  const changed = applicationChanges(preview).some(c => JSON.stringify(c.before) !== JSON.stringify(c.after));
  if (!version(value?.lockVersion) || value.lockVersion < preview.expectedPreferencesVersion + Number(changed) || !actual || typeof actual !== "object"
    || Object.keys(actual).length !== Object.keys(expected).length
    || Object.entries(expected).some(([k, v]) => k === "vadOptions" ? !sameOptionalVad(v, (actual as Record<string, unknown>)[k]) : JSON.stringify(v) !== JSON.stringify((actual as Record<string, unknown>)[k]))) fail();
}
