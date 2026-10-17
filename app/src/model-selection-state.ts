import type { OfficialModelOption } from "./model-download-state";
import type { ModelPlacementEntry } from "./model-inspection-state";
import { canReviewApplication, type ModelProfileView } from "./model-profile-state.ts";

const modelRoles = new Set(["llm", "asr", "vad", "segmentation", "speaker_embedding"]);
const retiredRecipes = new Set(["36416d1eeca501fdc083e2b31c9f49b4258342174b0f7d52798aa14b827dc44e"]);
export const isRetiredModel = (entry: ModelPlacementEntry) => entry.download != null && retiredRecipes.has(entry.download.recipeHash);
function identity(engine: string, files: { role: string; size: number; sha256: string }[]): string | null {
  const models = files.filter(file => modelRoles.has(file.role) && !(engine === "whisper.cpp" && file.role === "vad"));
  if (!models.length || models.some(file => !Number.isSafeInteger(file.size) || file.size <= 0 || !/^[0-9a-f]{64}$/u.test(file.sha256))) return null;
  return JSON.stringify([engine, models.map(file => [file.role, file.size, file.sha256]).sort((a, b) => String(a[0]).localeCompare(String(b[0])))]);
}
function entryIdentity(entry: ModelPlacementEntry): string {
  return identity(entry.engine, entry.modelFiles ?? []) ?? JSON.stringify([entry.engine, entry.download?.recipeHash ?? entry.packageId]);
}
export function matchesOfficialModel(entry: ModelPlacementEntry, option: OfficialModelOption): boolean {
  if (entry.engine !== option.review.manifest.engine) return false;
  if (entry.sourceType === "download" && entry.download?.recipeHash === option.review.recipeHash) return true;
  const key = identity(entry.engine, entry.modelFiles ?? []);
  return key !== null && key === identity(option.review.manifest.engine, option.review.manifest.files.map(file => ({ role: file.role, size: file.expectedSize, sha256: file.expectedHash })));
}
// Matching weights retain the display identity. Whisper needs the current
// engine, and Sherpa needs the C API library added for voice registration.
export function canReuseOfficialModel(entry: ModelPlacementEntry, option: OfficialModelOption): boolean {
  return matchesOfficialModel(entry, option) && (!["whisper.cpp", "sherpa-onnx"].includes(entry.engine)
    || entry.sourceType === "download" && entry.download?.recipeHash === option.review.recipeHash);
}
export function modelDisplayName(entry: ModelPlacementEntry, options: OfficialModelOption[]): string {
  const official = options.find(option => matchesOfficialModel(entry, option));
  if (official) return official.label;
  return entry.quantization && !entry.modelName.includes(entry.quantization) ? `${entry.modelName} · ${entry.quantization}` : entry.modelName;
}
function preferred(entries: ModelPlacementEntry[], profiles: ModelProfileView[]): ModelPlacementEntry {
  const score = (entry: ModelPlacementEntry) =>
    (profiles.some(profile => profile.name === "standard" && profile.components.some(component => component.packageId === entry.packageId && component.engine === entry.engine)) ? 4 : 0)
    + (canReviewApplication(entry) ? 2 : 0) + (entry.sourceType === "download" ? 1 : 0);
  return entries.reduce((best, entry) => score(entry) > score(best) ? entry : best);
}
export function installedModelGroups(entries: ModelPlacementEntry[], profiles: ModelProfileView[]) {
  const groups = new Map<string, ModelPlacementEntry[]>();
  for (const entry of entries.filter(entry => entry.placementRecorded && entry.bodyRemoved !== true)) {
    const key = entryIdentity(entry), group = groups.get(key) ?? [];
    group.push(entry); groups.set(key, group);
  }
  return [...groups.values()].map(group => ({ entry: preferred(group, profiles), entries: group }));
}
export interface ModelChoice {
  value: string;
  label: string;
  option?: OfficialModelOption;
  entry?: ModelPlacementEntry;
  packageIds: string[];
}
export function modelChoices(engine: string, entries: ModelPlacementEntry[], options: OfficialModelOption[], profiles: ModelProfileView[]): ModelChoice[] {
  const groups = installedModelGroups(entries.filter(entry => !isRetiredModel(entry)), profiles);
  const official = options.filter(option => option.review.manifest.engine === engine);
  const result: ModelChoice[] = official.map(option => {
    const matching = groups.flatMap(group => group.entries).filter(entry => matchesOfficialModel(entry, option));
    const reusable = matching.filter(entry => canReuseOfficialModel(entry, option) && (entry.sourceType === "download" || canReviewApplication(entry)));
    return { value: `official:${option.id}`, label: option.label, option,
      entry: reusable.length ? preferred(reusable, profiles) : undefined, packageIds: matching.map(entry => entry.packageId) };
  });
  for (const group of groups) {
    if (group.entry.engine !== engine || group.entries.some(entry => official.some(option => matchesOfficialModel(entry, option)))) continue;
    result.push({ value: group.entry.packageId, label: modelDisplayName(group.entry, options), entry: group.entry, packageIds: group.entries.map(entry => entry.packageId) });
  }
  return result;
}
