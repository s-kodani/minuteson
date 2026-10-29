import type { ArchiveMember, BackendManifest, DownloadSource, PackageFile, PackageManifest, Recipe } from "./generated/contracts";
import { validateRecipe } from "./generated/validators.js";
import type { ModelInspectionReport, ModelPlacementReceipt } from "./model-inspection-state";

export const downloadId = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const hash = /^[0-9a-f]{64}$/u, maximum = 32 * 1024 ** 3;
type File = Omit<PackageFile, "expectedSize" | "expectedHash"> & { expectedSize: number; expectedHash: string };
export type DownloadManifest = Omit<PackageManifest, "schemaVersion" | "quantization" | "files" | "backend"> & { schemaVersion: 1; quantization: string | null; files: File[]; backend?: BackendManifest };
export type DownloadArchive = ArchiveMember & { format: "tar" };
export const bundledWhisperSource = "mon-bundled://whisper-v1.9.4/whisper-runtime.tar";
export const bundledAsrSource = "mon-bundled://asr-v1/asr-runtime.tar";
export const bundledWindowsSource = "mon-bundled://windows-runtime-v1/runtime.tar";
const isBundledSource = (url: string) => url === bundledWhisperSource || url === bundledWindowsSource || url === bundledAsrSource;
export type DownloadRecipe = Omit<Recipe, "schemaVersion" | "manifest" | "sources"> & { schemaVersion: 4; manifest: DownloadManifest; sources: (Omit<DownloadSource, "archive"> & { archive?: DownloadArchive })[] };
export type DownloadReview = Omit<DownloadRecipe, "schemaVersion"> & { recipeSchemaVersion: 4; downloadBytes: number; archiveBytes: number; bundledBytes?: number; recipeHash: string; totalBytes: number; publisherAuthenticity: "not_verified"; installed: false; ready: false };
export type DownloadReceipt = { operationId: string; recipeHash: string; directory: string; report: ModelInspectionReport & { manifest: DownloadManifest }; installed: false; ready: false };
export type DownloadReadback = { state: "incomplete" | "complete"; review: DownloadReview; receipt: DownloadReceipt | null };
export type DownloadCatalog = { supported: boolean; unknownEntries: number; entries: { operationId: string; recipeHash: string | null; modelName: string | null; modelVersion: string | null; totalBytes: number | null; state: "attention" | "incomplete" | "completion_recorded" }[] };
export type DownloadProgress = { operationId: string; recipeHash: string; receivedBytes: number; totalBytes: number };
export type ModelTokenLimits = { maxInputTokens: number; maxOutputTokens: number; maxContextTokens: number; tokenizer: "utf8_byte_bpe" };
export type OfficialModelOption = { id: string; label: string; description: string; definition: string; review: DownloadReview; tokenLimits: ModelTokenLimits | null };
function fail(): never { throw Error("取得の定義・操作・応答が一致しません。同じ操作の結果を確認してください。"); }
function object(value: unknown): Record<string, unknown> { if (!value || typeof value !== "object" || Array.isArray(value)) fail(); return value as Record<string, unknown>; }
function keys(value: Record<string, unknown>, allowed: string[]) { if (Object.keys(value).some(key => !allowed.includes(key))) fail(); }
function text(value: unknown, limit: number): string { if (typeof value !== "string" || !value.trim() || /[\u0000-\u001f\u007f]/u.test(value) || new TextEncoder().encode(value).length > limit) fail(); return value; }
function sha(value: unknown): string { if (typeof value !== "string" || !hash.test(value)) fail(); return value; }
function size(value: unknown, max = maximum): number { if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > max) fail(); return value; }
function unready(value: Record<string, unknown>) { if (value.installed !== false || value.ready !== false || value.publisherAuthenticity !== "not_verified") fail(); }
function identity(value: unknown): string { if (typeof value !== "string" || !downloadId.test(value)) fail(); return value; }
// This preserves Rust struct order for SHA binding. Native validation remains
// authoritative for OS, roles, portable paths, URLs and metadata byte budget.
export function canonicalRecipe(value: unknown): DownloadRecipe {
  if (!validateRecipe(value)) fail();
  const recipe = object(value); keys(recipe, ["schemaVersion", "manifest", "sources"]);
  const m = object(recipe.manifest); keys(m, ["schemaVersion", "engine", "engineVersion", "modelName", "modelVersion", "quantization", "licenseRef", "target", "files", "backend"]);
  if (recipe.schemaVersion !== 4 || m.schemaVersion !== 1 || !Array.isArray(m.files) || m.files.length < 1 || m.files.length > 64 || !Array.isArray(recipe.sources) || recipe.sources.length !== m.files.length) fail();
  const manifest: DownloadManifest = {
    schemaVersion: 1, engine: text(m.engine, 32) as DownloadManifest["engine"], engineVersion: text(m.engineVersion, 128), modelName: text(m.modelName, 256), modelVersion: text(m.modelVersion, 128),
    quantization: m.quantization == null ? null : text(m.quantization, 128), licenseRef: text(m.licenseRef, 4096), target: text(m.target, 32) as DownloadManifest["target"],
    files: m.files.map(value => { const f = object(value); keys(f, ["relativePath", "role", "expectedSize", "expectedHash"]); return { relativePath: text(f.relativePath, 4096), role: text(f.role, 32) as File["role"], expectedSize: size(f.expectedSize), expectedHash: sha(f.expectedHash) }; }),
  };
  if (m.backend != null) {
    const b = object(m.backend); keys(b, ["id", "files"]); const files = object(b.files);
    manifest.backend = { id: text(b.id, 128), files: Object.fromEntries(Object.keys(files).sort(utf8Order).map(key => [text(key, 64), text(files[key], 4096)])) };
  }
  const sources = recipe.sources.map((value, index) => {
    const s = object(value); keys(s, ["relativePath", "url", "archive"]);
    if (s.relativePath !== manifest.files[index].relativePath) fail();
    const result: DownloadRecipe["sources"][number] = { relativePath: text(s.relativePath, 4096), url: text(s.url, 4096) };
    if (s.archive != null) {
      const a = object(s.archive); keys(a, ["format", "expectedSize", "expectedHash", "memberPath"]);
      if (a.format !== "tar" || size(a.expectedSize) === 0) fail();
      const memberPath = text(a.memberPath, 4096);
      if (memberPath.split("/").some(part => !part || part === "." || part === "..") || /[\\:]/u.test(memberPath)) fail();
      result.archive = { format: "tar", expectedSize: size(a.expectedSize), expectedHash: sha(a.expectedHash), memberPath };
    }
    if (result.url === bundledWhisperSource) {
      const file = manifest.files[index], member = result.archive?.memberPath;
      if (manifest.engine !== "whisper.cpp" || manifest.engineVersion !== "v1.9.4" || manifest.target !== "macos_aarch64"
        || !(file.role === "executable" && member === "bin/whisper-cli" || file.role === "notice" && ["notices/whisper-MIT.txt", "build-info.json"].includes(member ?? ""))) fail();
    } else if (result.url === bundledAsrSource) {
      if (manifest.engine !== "asr" || manifest.target !== "macos_aarch64" || !result.archive || !["executable", "notice"].includes(manifest.files[index].role)) fail();
    } else if (result.url === bundledWindowsSource) {
      if (manifest.target !== "windows_x86_64" || !result.archive
        || !["executable", "runtime_library", "notice"].includes(manifest.files[index].role)) fail();
    } else if (result.url.startsWith("mon-bundled:")) fail();
    return result;
  });
  const result: DownloadRecipe = { schemaVersion: recipe.schemaVersion as 4, manifest, sources };
  if (sources.some(source => source.url === bundledWhisperSource)
    && !manifest.files.some((file, i) => file.role === "asr" && sources[i].url.startsWith("https://"))) fail();
  downloadByteTotals(result);
  size(manifest.files.reduce((n, f) => n + f.expectedSize, 0));
  if (new TextEncoder().encode(JSON.stringify(result)).length > 65_536) fail();
  return result;
}
export function downloadByteTotals(recipe: DownloadRecipe): { downloadBytes: number; archiveBytes: number; bundledBytes?: number } {
  let downloadBytes = 0, archiveBytes = 0, bundledBytes = 0;
  const archives = new Map<string, DownloadArchive>(), members = new Set<string>();
  recipe.sources.forEach((source, index) => {
    if (source.archive) {
      const previous = archives.get(source.url), key = JSON.stringify([source.url, source.archive.memberPath]);
      if (members.has(key) || previous && (previous.expectedHash !== source.archive.expectedHash || previous.expectedSize !== source.archive.expectedSize)) fail();
      members.add(key);
      if (!previous) {
        if (isBundledSource(source.url)) bundledBytes += source.archive.expectedSize;
        else downloadBytes += source.archive.expectedSize;
        archiveBytes += source.archive.expectedSize; archives.set(source.url, source.archive);
      }
    } else downloadBytes += recipe.manifest.files[index].expectedSize;
  });
  if (recipe.sources.some(source => !source.archive && archives.has(source.url))) fail();
  return { downloadBytes: size(downloadBytes), archiveBytes: size(archiveBytes), ...(recipe.sources.some(source => isBundledSource(source.url)) ? { bundledBytes: size(bundledBytes) } : {}) };
}
export function downloadTransferBytes(review: DownloadReview): number { return review.downloadBytes; }
export function parseDownloadDefinition(definition: string): DownloadRecipe {
  if (new TextEncoder().encode(definition).length > 65_536) fail();
  return canonicalRecipe(JSON.parse(definition));
}
const utf8 = new TextEncoder();
function utf8Order(a: string, b: string): number {
  const x = utf8.encode(a), y = utf8.encode(b);
  for (let i = 0; i < Math.min(x.length, y.length); i++) if (x[i] !== y[i]) return x[i] - y[i];
  return x.length - y.length;
}
/** Keep Rust struct field order; only BackendManifest.files is a BTreeMap.
 * Numeric keys and non-BMP keys cannot rely on JavaScript object enumeration. */
export function canonicalModelJSON(value: unknown): string {
  const write = (value: unknown, backend = false, map = false): string => {
    if (Array.isArray(value)) return `[${value.map(item => write(item)).join(",")}]`;
    if (value && typeof value === "object") {
      const object = value as Record<string, unknown>, keys = Object.keys(object).filter(key => object[key] !== undefined);
      if (map) keys.sort(utf8Order);
      return `{${keys.map(key => `${JSON.stringify(key)}:${write(object[key], key === "backend", backend && key === "files")}`).join(",")}}`;
    }
    return JSON.stringify(value);
  };
  return write(value);
}
export async function downloadDigest(value: unknown): Promise<string> {
  const bytes = new TextEncoder().encode(canonicalModelJSON(value));
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), byte => byte.toString(16).padStart(2, "0")).join("");
}
export async function assertDownloadReview(value: unknown, expectedRecipeHash?: string): Promise<DownloadReview> {
  const review = object(value); unready(review);
  if (review.recipeSchemaVersion !== 4) fail();
  const recipe = canonicalRecipe({ schemaVersion: review.recipeSchemaVersion, manifest: review.manifest, sources: review.sources });
  const recipeHash = await downloadDigest(recipe), totalBytes = recipe.manifest.files.reduce((n, f) => n + f.expectedSize, 0);
  if (review.recipeHash !== recipeHash || (expectedRecipeHash !== undefined && recipeHash !== expectedRecipeHash) || review.totalBytes !== totalBytes) fail();
  const totals = downloadByteTotals(recipe);
  if (review.downloadBytes !== totals.downloadBytes || review.archiveBytes !== totals.archiveBytes) fail();
  if (review.bundledBytes !== totals.bundledBytes) fail();
  return { manifest: recipe.manifest, sources: recipe.sources, recipeHash, totalBytes, recipeSchemaVersion: recipe.schemaVersion, ...totals, publisherAuthenticity: "not_verified", installed: false, ready: false };
}
export async function assertOfficialModelOptions(value: unknown): Promise<OfficialModelOption[]> {
  if (!Array.isArray(value) || value.length > 16) fail();
  const seen = new Set<string>();
  const result: OfficialModelOption[] = [];
  for (const entry of value) {
    const option = object(entry); keys(option, ["id", "label", "description", "definition", "review", "tokenLimits"]);
    const id = text(option.id, 128);
    if (!/^[a-z0-9-]+$/u.test(id) || seen.has(id)) fail(); seen.add(id);
    const definition = text(option.definition, 65_536), recipe = parseDownloadDefinition(definition);
    let tokenLimits: ModelTokenLimits | null = null;
    if (recipe.manifest.engine === "llama.cpp") {
      const limits = object(option.tokenLimits); keys(limits, ["maxInputTokens", "maxOutputTokens", "maxContextTokens", "tokenizer"]);
      const integer = (n: unknown, min: number, max: number): n is number => typeof n === "number" && Number.isSafeInteger(n) && n >= min && n <= max;
      if (!integer(limits.maxContextTokens, 2048, 1048576) || !integer(limits.maxInputTokens, 2048, limits.maxContextTokens)
        || !integer(limits.maxOutputTokens, 64, limits.maxContextTokens) || limits.tokenizer !== "utf8_byte_bpe") fail();
      tokenLimits = { maxInputTokens: limits.maxInputTokens as number, maxOutputTokens: limits.maxOutputTokens as number,
        maxContextTokens: limits.maxContextTokens as number, tokenizer: "utf8_byte_bpe" };
    } else if (option.tokenLimits != null) fail();
    result.push({ id, label: text(option.label, 128), description: text(option.description, 1024), definition,
      review: await assertDownloadReview(option.review, await downloadDigest(recipe)), tokenLimits });
  }
  return result;
}
export async function assertDownloadReceipt(value: unknown, operationId: string, review: DownloadReview): Promise<DownloadReceipt> {
  const receipt = object(value), report = object(receipt.report); unready(report);
  if (receipt.operationId !== identity(operationId) || receipt.recipeHash !== review.recipeHash || receipt.installed !== false || receipt.ready !== false || report.engineCompatibility !== "not_verified" || report.totalBytes !== review.totalBytes || !Array.isArray(report.files) || report.files.length !== review.manifest.files.length) fail();
  const manifest = canonicalRecipe({ schemaVersion: review.recipeSchemaVersion, manifest: report.manifest, sources: review.sources }).manifest;
  if (JSON.stringify(manifest) !== JSON.stringify(review.manifest)) fail();
  const files = report.files.map((value, index) => {
    const f = object(value), expected = review.manifest.files[index];
    if (f.relativePath !== expected.relativePath || f.role !== expected.role || f.actualSize !== expected.expectedSize || f.actualHash !== expected.expectedHash || f.expectedSizeChecked !== true || f.expectedHashChecked !== true) fail();
    return { relativePath: expected.relativePath, role: expected.role, actualSize: expected.expectedSize, actualHash: expected.expectedHash, expectedSizeChecked: true, expectedHashChecked: true };
  });
  const manifestHash = await downloadDigest(manifest);
  if (report.manifestHash !== manifestHash || report.reviewHash !== await downloadDigest([1, manifest, manifestHash, files])) fail();
  text(receipt.directory, 16_384); // Display only. Never used as a placement path.
  return structuredClone(value) as DownloadReceipt;
}
export async function assertDownloadReadback(value: unknown, operationId: string, recipeHash: string): Promise<DownloadReadback> {
  const result = object(value), review = await assertDownloadReview(result.review, recipeHash);
  if (result.state === "incomplete" && result.receipt === null) return { state: "incomplete", review, receipt: null };
  if (result.state !== "complete") fail();
  return { state: "complete", review, receipt: await assertDownloadReceipt(result.receipt, operationId, review) };
}
export function assertDownloadCatalog(value: unknown): DownloadCatalog {
  const catalog = object(value);
  if (typeof catalog.supported !== "boolean" || !Array.isArray(catalog.entries) || catalog.entries.length > 4096) fail();
  size(catalog.unknownEntries, 4096);
  if (!catalog.supported && (catalog.entries.length || catalog.unknownEntries !== 0)) fail();
  const seen = new Set<string>();
  for (const value of catalog.entries) {
    const entry = object(value), id = identity(entry.operationId);
    if (seen.has(id) || !["attention", "incomplete", "completion_recorded"].includes(String(entry.state))) fail();
    seen.add(id);
    if (entry.recipeHash !== null) sha(entry.recipeHash);
    if (entry.modelName !== null) text(entry.modelName, 256);
    if (entry.modelVersion !== null) text(entry.modelVersion, 128);
    if (entry.totalBytes !== null) size(entry.totalBytes);
    if (entry.state !== "attention" && [entry.recipeHash, entry.modelName, entry.modelVersion, entry.totalBytes].some(v => v === null)) fail();
  }
  return structuredClone(value) as DownloadCatalog;
}
export function assertDownloadLifetime(value: unknown): { active: boolean; cancelled: boolean } {
  const result = object(value);
  if (typeof result.active !== "boolean" || typeof result.cancelled !== "boolean") fail();
  return { active: result.active, cancelled: result.cancelled };
}
export function acceptDownloadProgress(value: unknown, operationId: string, review: DownloadReview, previous: number): number | null {
  try {
    const progress = object(value);
    if (progress.operationId !== operationId || progress.recipeHash !== review.recipeHash || progress.totalBytes !== downloadTransferBytes(review)) return null;
    const received = size(progress.receivedBytes, downloadTransferBytes(review));
    return received >= previous ? received : null;
  } catch { return null; }
}
export function assertDownloadCopyResult(value: unknown, packageId: string, reviewHash: string, totalBytes: number): { active: boolean; cancelled: boolean; receipt: ModelPlacementReceipt | null } {
  const result = object(value), lifetime = assertDownloadLifetime(value);
  if (result.receipt === null) return { ...lifetime, receipt: null };
  const receipt = object(result.receipt);
  if (receipt.packageId !== identity(packageId) || receipt.reviewHash !== sha(reviewHash) || receipt.totalBytes !== totalBytes || typeof receipt.placementRecorded !== "boolean" || receipt.ready !== false || receipt.engineCompatibility !== "not_verified") fail();
  return { ...lifetime, receipt: structuredClone(receipt) as unknown as ModelPlacementReceipt };
}
