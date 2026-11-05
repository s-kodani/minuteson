import type { OfficialModelOption } from "./model-download-state";
import type { ModelPlacementEntry } from "./model-inspection-state";
import { canReviewApplication, type ModelProfileComponent, type ModelProfileView } from "./model-profile-state.ts";

export const isAsrEngine = (engine: string) => engine === "whisper.cpp" || engine === "asr";
const matchesSlot = (engine: string, slot: string) => slot === "asr" ? isAsrEngine(engine) : engine === slot;
const modelRoles = new Set(["llm", "asr", "asr_asset", "vad", "segmentation", "speaker_embedding"]);
function identity(engine: string, files: { role: string; size: number; sha256: string }[], backendId?: string): string | null {
  if (engine === "asr" && !backendId) return null;
  const models = files.filter(file => modelRoles.has(file.role) && !(engine === "whisper.cpp" && file.role === "vad"));
  if (!models.length || models.some(file => !Number.isSafeInteger(file.size) || file.size <= 0 || !/^[0-9a-f]{64}$/u.test(file.sha256))) return null;
  return JSON.stringify([engine, backendId ?? null, models.map(file => [file.role, file.size, file.sha256]).sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)))]);
}
function entryIdentity(entry: ModelPlacementEntry): string {
  return identity(entry.engine, entry.modelFiles ?? [], entry.backendId) ?? JSON.stringify([entry.engine, entry.download?.recipeHash ?? entry.packageId]);
}
export function matchesOfficialModel(entry: ModelPlacementEntry, option: OfficialModelOption): boolean {
  if (entry.engine !== option.review.manifest.engine || entry.engine === "asr" && entry.backendId !== option.review.manifest.backend?.id) return false;
  if (entry.sourceType === "download" && entry.download?.recipeHash === option.review.recipeHash) return true;
  const key = identity(entry.engine, entry.modelFiles ?? [], entry.backendId);
  return key !== null && key === identity(option.review.manifest.engine, option.review.manifest.files.map(file => ({ role: file.role, size: file.expectedSize, sha256: file.expectedHash })), option.review.manifest.backend?.id);
}
// Matching weights retain the display identity. Reusing downloaded packages
// requires every runtime/model/notice byte to match the current manifest.
export function canReuseOfficialModel(entry: ModelPlacementEntry, option: OfficialModelOption): boolean {
  return matchesOfficialModel(entry, option) && entry.sourceType === "download"
    && (entry.download?.recipeHash === option.review.recipeHash
      || /^[0-9a-f]{64}$/u.test(option.review.packageReviewHash) && entry.reviewHash === option.review.packageReviewHash);
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
  return [...groups].map(([key, group]) => ({ key, entry: preferred(group, profiles), entries: group,
    totalBytes: group.reduce((sum, entry) => sum + entry.totalBytes, 0) }));
}
export interface ModelChoice {
  value: string;
  label: string;
  option?: OfficialModelOption;
  entry?: ModelPlacementEntry;
  packageIds: string[];
}
function containsModelFiles(entry: ModelPlacementEntry, option: OfficialModelOption, role?: string): boolean {
  if (entry.sourceType === "download" && entry.download?.recipeHash === option.review.recipeHash) return true;
  const required = option.review.manifest.files.filter(file => role ? file.role === role : modelRoles.has(file.role));
  return required.length > 0 && required.every(file => entry.modelFiles?.some(installed =>
    installed.role === file.role && installed.size === file.expectedSize && installed.sha256 === file.expectedHash));
}
export type ModelChoiceAction = "in_use" | "add_vad" | "use" | "update_runtime" | "download";
// An applied, checked package is independent of a new catalog's transport
// archive. A rebuilt shared archive must not erase the current configuration.
// Runtime changes still require the full-package and compatibility checks in
// canReuseOfficialModel / useModel, even if the model weights are unchanged.
export function modelChoiceAction(choice: ModelChoice | undefined, entries: ModelPlacementEntry[], used?: ModelProfileComponent): ModelChoiceAction {
  if (!choice) return "download";
  const option = choice.option;
  const installed = entries.filter(entry => choice.packageIds.includes(entry.packageId) && entry.placementRecorded && entry.bodyRemoved !== true);
  const applied = installed.find(entry => entry.packageId === used?.packageId && entry.engine === used.engine);
  if (applied && used?.settingsMatch && used.compatibilityCurrent && canReviewApplication(applied)
    && applied.compatibility?.operationId === used.checkOperationId
    && (!option || containsModelFiles(applied, option) && canReuseOfficialModel(applied, option))) return "in_use";
  if (choice.entry && installed.some(entry => entry.packageId === choice.entry!.packageId && (!option || containsModelFiles(entry, option)))) return "use";
  if (option?.review.manifest.engine === "whisper.cpp"
    && option.review.manifest.files.some(file => file.role === "vad")
    && installed.some(entry => entry.modelFiles?.some(file => file.role === "asr"))
    && !installed.some(entry => containsModelFiles(entry, option, "vad"))) return "add_vad";
  return installed.length ? "update_runtime" : "download";
}
export function modelChoices(engine: string, entries: ModelPlacementEntry[], options: OfficialModelOption[], profiles: ModelProfileView[]): ModelChoice[] {
  const groups = installedModelGroups(entries, profiles);
  const official = options.filter(option => matchesSlot(option.review.manifest.engine, engine));
  const result: ModelChoice[] = official.map(option => {
    const matching = groups.flatMap(group => group.entries).filter(entry => matchesOfficialModel(entry, option));
    const reusable = matching.filter(entry => canReuseOfficialModel(entry, option) && (entry.sourceType === "download" || canReviewApplication(entry)));
    return { value: `official:${option.id}`, label: option.label, option,
      entry: reusable.length ? preferred(reusable, profiles) : undefined, packageIds: matching.map(entry => entry.packageId) };
  });
  for (const group of groups) {
    if (!matchesSlot(group.entry.engine, engine) || group.entries.some(entry => official.some(option => matchesOfficialModel(entry, option)))) continue;
    result.push({ value: group.entry.packageId, label: modelDisplayName(group.entry, options), entry: group.entry, packageIds: group.entries.map(entry => entry.packageId) });
  }
  return result;
}
