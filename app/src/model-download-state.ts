import type { ModelInspectionReport, ModelPlacementReceipt } from "./model-inspection-state";

export const downloadId = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const hash = /^[0-9a-f]{64}$/u, maximum = 32 * 1024 ** 3;
type File = { relativePath: string; role: string; expectedSize: number; expectedHash: string };
export type DownloadManifest = { schemaVersion: 1; engine: string; engineVersion: string; modelName: string; modelVersion: string; quantization: string | null; licenseRef: string; target: string; files: File[] };
export type DownloadRecipe = { schemaVersion: 1; manifest: DownloadManifest; sources: { relativePath: string; url: string }[] };
export type DownloadReview = Omit<DownloadRecipe, "schemaVersion"> & { recipeHash: string; totalBytes: number; publisherAuthenticity: "not_verified"; installed: false; ready: false };
export type DownloadReceipt = { operationId: string; recipeHash: string; directory: string; report: ModelInspectionReport & { manifest: DownloadManifest }; installed: false; ready: false };
export type DownloadReadback = { state: "incomplete" | "complete"; review: DownloadReview; receipt: DownloadReceipt | null };
export type DownloadCatalog = { supported: boolean; unknownEntries: number; entries: { operationId: string; recipeHash: string | null; modelName: string | null; modelVersion: string | null; totalBytes: number | null; state: "attention" | "incomplete" | "completion_recorded" }[] };
export type DownloadProgress = { operationId: string; recipeHash: string; receivedBytes: number; totalBytes: number };
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
  const recipe = object(value); keys(recipe, ["schemaVersion", "manifest", "sources"]);
  const m = object(recipe.manifest); keys(m, ["schemaVersion", "engine", "engineVersion", "modelName", "modelVersion", "quantization", "licenseRef", "target", "files"]);
  if (recipe.schemaVersion !== 1 || m.schemaVersion !== 1 || !Array.isArray(m.files) || m.files.length < 1 || m.files.length > 64 || !Array.isArray(recipe.sources) || recipe.sources.length !== m.files.length) fail();
  const manifest: DownloadManifest = {
    schemaVersion: 1, engine: text(m.engine, 32), engineVersion: text(m.engineVersion, 128), modelName: text(m.modelName, 256), modelVersion: text(m.modelVersion, 128),
    quantization: m.quantization == null ? null : text(m.quantization, 128), licenseRef: text(m.licenseRef, 4096), target: text(m.target, 32),
    files: m.files.map(value => { const f = object(value); keys(f, ["relativePath", "role", "expectedSize", "expectedHash"]); return { relativePath: text(f.relativePath, 4096), role: text(f.role, 32), expectedSize: size(f.expectedSize), expectedHash: sha(f.expectedHash) }; }),
  };
  const sources = recipe.sources.map((value, index) => {
    const s = object(value); keys(s, ["relativePath", "url"]);
    if (s.relativePath !== manifest.files[index].relativePath) fail();
    return { relativePath: text(s.relativePath, 4096), url: text(s.url, 4096) };
  });
  const result: DownloadRecipe = { schemaVersion: 1, manifest, sources };
  size(manifest.files.reduce((n, f) => n + f.expectedSize, 0));
  if (new TextEncoder().encode(JSON.stringify(result)).length > 65_536) fail();
  return result;
}
export function parseDownloadDefinition(definition: string): DownloadRecipe {
  if (new TextEncoder().encode(definition).length > 65_536) fail();
  return canonicalRecipe(JSON.parse(definition));
}
export async function downloadDigest(value: unknown): Promise<string> {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), byte => byte.toString(16).padStart(2, "0")).join("");
}
export async function assertDownloadReview(value: unknown, expectedRecipeHash?: string): Promise<DownloadReview> {
  const review = object(value); unready(review);
  const recipe = canonicalRecipe({ schemaVersion: 1, manifest: review.manifest, sources: review.sources });
  const recipeHash = await downloadDigest(recipe), totalBytes = recipe.manifest.files.reduce((n, f) => n + f.expectedSize, 0);
  if (review.recipeHash !== recipeHash || (expectedRecipeHash !== undefined && recipeHash !== expectedRecipeHash) || review.totalBytes !== totalBytes) fail();
  return { manifest: recipe.manifest, sources: recipe.sources, recipeHash, totalBytes, publisherAuthenticity: "not_verified", installed: false, ready: false };
}
export async function assertDownloadReceipt(value: unknown, operationId: string, review: DownloadReview): Promise<DownloadReceipt> {
  const receipt = object(value), report = object(receipt.report); unready(report);
  if (receipt.operationId !== identity(operationId) || receipt.recipeHash !== review.recipeHash || receipt.installed !== false || receipt.ready !== false || report.engineCompatibility !== "not_verified" || report.totalBytes !== review.totalBytes || !Array.isArray(report.files) || report.files.length !== review.manifest.files.length) fail();
  const manifest = canonicalRecipe({ schemaVersion: 1, manifest: report.manifest, sources: review.sources }).manifest;
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
    if (progress.operationId !== operationId || progress.recipeHash !== review.recipeHash || progress.totalBytes !== review.totalBytes) return null;
    const received = size(progress.receivedBytes, review.totalBytes);
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
