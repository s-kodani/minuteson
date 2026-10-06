import type { EnginePreferences } from "./analysis-state.ts";
import type { DiarizationEngine } from "./diarization-state.ts";
import type { MinutesEngine } from "./minutes-state.ts";
import { modelBodyAvailable, placementLabel, type ModelPlacementEntry } from "./model-inspection-state.ts";

export type ModelProfileName = "standard" | "high_accuracy";
export type ModelSettings = { engine: "whisper.cpp"; settings: EnginePreferences }
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
}
const id = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(v);
const sha = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{64}$/.test(v);
const version = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const names = ["standard", "high_accuracy"];
const roles = { "whisper.cpp": ["asr"], "sherpa-onnx": ["segmentation", "speaker_embedding"], "llama.cpp": ["llm"] };
const fileKeys = ["executablePath", "modelPath", "vadModelPath", "segmentationModelPath", "embeddingModelPath", "runtimeLibraryPaths"];
const keys = {
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
    if (!component || !Object.hasOwn(roles, component.engine) || engines.has(component.engine)
      || !id(component.packageId) || !id(component.checkOperationId)
      || typeof component.modelName !== "string" || !component.modelName || typeof component.modelVersion !== "string" || !component.modelVersion
      || typeof component.settingsMatch !== "boolean" || typeof component.compatibilityCurrent !== "boolean"
      || JSON.stringify(component.roles) !== JSON.stringify(roles[component.engine])) fail();
    engines.add(component.engine);
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
  if (entries.length !== keys[value.engine].length || keys[value.engine].some(k => !Object.hasOwn(value.settings, k))) fail();
  if (entries.some(([k, v]) => fileKeys.includes(k)
    ? (k === "runtimeLibraryPaths" ? !Array.isArray(v) || v.some(p => typeof p !== "string") : k === "vadModelPath" && v === null ? false : typeof v !== "string")
    : k === "useGpu" ? typeof v !== "boolean" : typeof v !== "number" || !Number.isSafeInteger(v) || v < 0)) fail();
  if (value.settings.threads < 1 || value.settings.threads > 64) fail();
  return entries;
}
export function applicationChanges(preview: ModelApplicationPreview) {
  const before = new Map(settingsEntries(preview.before));
  return settingsEntries(preview.after).map(([key, after]) => ({ key, before: before.get(key), after, file: fileKeys.includes(key) }));
}
export function assertApplicationPreview(preview: ModelApplicationPreview, entry: ModelPlacementEntry, name: ModelProfileName, current: ModelProfileView) {
  if (!canReviewApplication(entry) || !preview || preview.schemaVersion !== 1 || preview.profile !== name || current.name !== name
    || preview.packageId !== entry.packageId || preview.reviewHash !== entry.reviewHash || preview.checkOperationId !== entry.compatibility?.operationId
    || !sha(preview.reviewDigest) || !version(preview.expectedPreferencesVersion) || !version(preview.expectedProfileVersion)
    || preview.expectedProfileId !== current.id || preview.expectedProfileVersion !== current.profileVersion
    || (current.id === null ? preview.expectedProfileHash !== null : !sha(preview.expectedProfileHash))
    || preview.before.engine !== entry.engine || preview.after.engine !== entry.engine) fail();
  assertProfileView(current);
  for (const change of applicationChanges(preview)) {
    if (!change.file && change.before !== change.after) fail();
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
export const preferenceCommand = (engine: ModelSettings["engine"]) => ({ "whisper.cpp": "get_analysis_preferences", "sherpa-onnx": "get_diarization_preferences", "llama.cpp": "get_minutes_preferences" })[engine];
export function assertAppliedPreferences(value: { lockVersion: number; payload: { standard: unknown; highAccuracy: unknown } }, preview: ModelApplicationPreview) {
  const key = preview.profile === "standard" ? "standard" : "highAccuracy";
  const expected = Object.fromEntries(settingsEntries(preview.after));
  const actual = value?.payload?.[key];
  const changed = applicationChanges(preview).some(c => JSON.stringify(c.before) !== JSON.stringify(c.after));
  if (!version(value?.lockVersion) || value.lockVersion < preview.expectedPreferencesVersion + Number(changed) || !actual || typeof actual !== "object"
    || Object.keys(actual).length !== Object.keys(expected).length
    || Object.entries(expected).some(([k, v]) => JSON.stringify(v) !== JSON.stringify((actual as Record<string, unknown>)[k]))) fail();
}
