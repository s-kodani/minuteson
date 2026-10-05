import type { ModelPlacementEntry } from "./model-inspection-state";

// A point-in-time, read-only observation. It is not persistent consent or a
// deletion permit, and must never be sent as authority to a mutation command.
export interface ModelDeletionEntry {
  relativePath: string;
  directory: boolean;
  registered: boolean;
  present: boolean;
  bytes: number | null;
  sha256: string | null;
  changed: boolean;
}
export interface ModelDeletionPreview {
  schemaVersion: 1;
  inspectionId: string;
  packageId: string;
  reviewHash: string;
  censusHash: string;
  modelName: string;
  modelVersion: string;
  modelsRoot: string;
  entries: ModelDeletionEntry[];
  bytes: number;
  missingFiles: number;
  changedFiles: number;
  unregisteredFiles: number;
  profileReferences: number;
  jobReferences: number;
  activeJobs: number;
  executeAllowed: false;
}
const id = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const hash = /^[0-9a-f]{64}$/;
const encoder = new TextEncoder();
const number = (n: unknown): n is number => typeof n === "number" && Number.isSafeInteger(n) && n >= 0;
const text = (s: unknown, max: number): s is string => typeof s === "string" && s.length > 0 && !s.includes("\0") && encoder.encode(s).length <= max;
const object = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === "object" && !Array.isArray(v);
function pathValid(path: unknown): path is string {
  return typeof path === "string" && encoder.encode(path).length <= 4096 && !path.includes("\0") && !path.includes("\\") && !/^[A-Za-z]:/.test(path) &&
    (path === "" || path.split("/").length <= 32 && path.split("/").every(p => p !== "" && p !== "." && p !== ".." && encoder.encode(p).length <= 255));
}
// Rust's BTreeMap sorts UTF-8 bytes, not JavaScript UTF-16 code units.
function compare(a: string, b: string): number {
  const left = encoder.encode(a), right = encoder.encode(b);
  for (let i = 0; i < Math.min(left.length, right.length); i++) if (left[i] !== right[i]) return left[i] - right[i];
  return left.length - right.length;
}
export function assertModelDeletionPreview(value: unknown, token: string, entry: Pick<ModelPlacementEntry, "packageId" | "reviewHash" | "modelName" | "modelVersion">): asserts value is ModelDeletionPreview {
  const fail = (): never => { throw Error("モデル削除の対象検査と選択した配置・操作が一致しません。削除せず、配置記録を再読込してください。"); };
  if (!object(value) || value.schemaVersion !== 1 || value.executeAllowed !== false || !id.test(token) || value.inspectionId !== token ||
    !text(value.packageId, 36) || !id.test(value.packageId) || value.packageId !== entry.packageId || !text(value.reviewHash, 64) || !hash.test(value.reviewHash) || value.reviewHash !== entry.reviewHash ||
    !text(value.censusHash, 64) || !hash.test(value.censusHash) || !text(value.modelName, 4096) || value.modelName !== entry.modelName || !text(value.modelVersion, 4096) || value.modelVersion !== entry.modelVersion ||
    !text(value.modelsRoot, 4096) || !value.modelsRoot.startsWith("/") || value.modelsRoot.split("/").some(p => p === "." || p === "..") || !value.modelsRoot.endsWith("/models") || value.modelsRoot.includes("\\") || value.modelsRoot.includes("//") ||
    !Array.isArray(value.entries) || value.entries.length === 0 || value.entries.length > 256 ||
    !["bytes", "missingFiles", "changedFiles", "unregisteredFiles", "profileReferences", "jobReferences", "activeJobs"].every(key => number(value[key]))) fail();
  const preview = value as unknown as ModelDeletionPreview;
  const paths = new Map<string, ModelDeletionEntry>();
  let bytes = 0, missing = 0, changed = 0, unknown = 0, previous: string | null = null;
  for (const item of preview.entries) {
    if (!object(item) || !pathValid(item.relativePath) || !["directory", "registered", "present", "changed"].every(key => typeof item[key] === "boolean") ||
      previous !== null && compare(previous, item.relativePath) >= 0) fail();
    if (item.directory || !item.present) {
      if (item.bytes !== null || item.sha256 !== null || item.changed) fail();
    } else {
      const amount = item.bytes;
      if (!number(amount) || typeof item.sha256 !== "string" || !hash.test(item.sha256) || item.changed && !item.registered) return fail();
      bytes += amount;
      if (!number(bytes)) fail();
    }
    if (!item.registered && !item.present) fail();
    if (!item.directory && !item.present) missing++;
    if (item.changed) changed++;
    if (!item.directory && !item.registered) unknown++;
    if (item.relativePath === "") {
      if (!item.directory || !item.registered) fail();
    } else {
      const parent = paths.get(item.relativePath.includes("/") ? item.relativePath.slice(0, item.relativePath.lastIndexOf("/")) : "");
      if (!parent?.directory || item.present && !parent.present) fail();
    }
    paths.set(item.relativePath, item);
    previous = item.relativePath;
  }
  if (!paths.has("") || bytes !== preview.bytes || missing !== preview.missingFiles || changed !== preview.changedFiles || unknown !== preview.unregisteredFiles) fail();
}
export function modelDeletionEntryLabel(entry: ModelDeletionEntry): string {
  if (!entry.present) return "欠落（配置記録のみ）";
  if (!entry.registered) return entry.directory ? "未登録フォルダー" : "未登録ファイル";
  if (entry.changed) return "登録内容から変更あり";
  return entry.directory ? "登録フォルダー" : "登録内容と一致";
}
