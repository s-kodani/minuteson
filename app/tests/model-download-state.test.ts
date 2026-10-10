import test from "node:test";
import assert from "node:assert/strict";
import { canonicalRecipe, parseDownloadDefinition, downloadDigest, assertDownloadReview, assertDownloadReceipt, assertDownloadReadback, assertDownloadCatalog, assertDownloadLifetime, acceptDownloadProgress, assertDownloadCopyResult } from "../src/model-download-state.ts";
import { placementLabel } from "../src/model-inspection-state.ts";
const id = "11111111-1111-4111-8111-111111111111", other = "22222222-2222-4222-8222-222222222222", hash = "a".repeat(64);
const recipe = canonicalRecipe({ schemaVersion: 1, manifest: { schemaVersion: 1, engine: "whisper.cpp", engineVersion: "test", modelName: "合成素材", modelVersion: "1", licenseRef: "fixture only", target: "macos_aarch64", files: [{ relativePath: "cli", role: "executable", expectedSize: 16, expectedHash: hash }, { relativePath: "model", role: "asr", expectedSize: 17, expectedHash: hash }] }, sources: [{ relativePath: "cli", url: "https://example.com/cli" }, { relativePath: "model", url: "https://example.com/model" }] });
async function review() { return assertDownloadReview({ manifest: recipe.manifest, sources: recipe.sources, recipeHash: await downloadDigest(recipe), totalBytes: 33, installed: false, ready: false, publisherAuthenticity: "not_verified" }); }
async function receipt() {
  const checked = await review(), manifestHash = await downloadDigest(recipe.manifest);
  const files = recipe.manifest.files.map(file => ({ relativePath: file.relativePath, role: file.role, actualSize: file.expectedSize, actualHash: file.expectedHash, expectedSizeChecked: true, expectedHashChecked: true }));
  return { operationId: id, recipeHash: checked.recipeHash, directory: "/synthetic/payload", installed: false, ready: false, report: { manifest: recipe.manifest, manifestHash, files, totalBytes: 33, reviewHash: await downloadDigest([1, recipe.manifest, manifestHash, files]), engineCompatibility: "not_verified", publisherAuthenticity: "not_verified", installed: false, ready: false } };
}
test("recipe canonical struct order and optional null bind regardless of input property order", async () => {
  const reversed = Object.fromEntries(Object.entries(recipe).reverse());
  assert.equal(await downloadDigest(canonicalRecipe(reversed)), await downloadDigest(recipe));
  assert.equal(canonicalRecipe(recipe).manifest.quantization, null);
  assert.equal(JSON.stringify(parseDownloadDefinition(JSON.stringify(recipe))), JSON.stringify(recipe));
});
test("canonical unicode recipe agrees with native Rust independent golden", async () => {
  const value = structuredClone(recipe);
  value.manifest.modelName = "合成取得素材"; value.manifest.licenseRef = "synthetic fixture only"; value.manifest.files[1].expectedHash = "b".repeat(64);
  assert.equal(await downloadDigest(value), "83d86a1f28ad309e2d0e0ceb70ba728541d8aec29a1963c2a8ceda0b23551226");
});
for (const field of ["schema", "missing-size", "missing-hash", "source-order", "total", "unknown", "byte-limit"]) test(`definition rejects ${field} before review`, () => {
  const value = structuredClone(recipe);
  if (field === "schema") value.schemaVersion = 2 as 1;
  else if (field === "missing-size") value.manifest.files[0].expectedSize = null as unknown as number;
  else if (field === "missing-hash") value.manifest.files[0].expectedHash = "";
  else if (field === "source-order") value.sources.reverse();
  else if (field === "total") value.manifest.files[0].expectedSize = 32 * 1024 ** 3;
  else if (field === "unknown") Object.assign(value, { execute: true });
  else value.sources[0].url = "x".repeat(65_536);
  assert.throws(() => parseDownloadDefinition(JSON.stringify(value)));
});
for (const field of ["hash", "total", "ready", "installed", "authenticity", "original"]) test(`review refuses mismatched ${field}`, async () => {
  const value = await review();
  if (field === "hash") value.recipeHash = hash;
  else if (field === "total") value.totalBytes++;
  else if (field === "ready") Object.assign(value, { ready: true });
  else if (field === "installed") Object.assign(value, { installed: true });
  else if (field === "authenticity") Object.assign(value, { publisherAuthenticity: "verified" });
  await assert.rejects(assertDownloadReview(value, field === "original" ? hash : undefined));
});
test("exact receipt and complete/incomplete readbacks never grant installed or ready", async () => {
  const checked = await review(), result = await assertDownloadReceipt(await receipt(), id, checked);
  assert.equal(result.ready, false);
  assert.equal((await assertDownloadReadback({ state: "complete", review: checked, receipt: result }, id, checked.recipeHash)).state, "complete");
  assert.equal((await assertDownloadReadback({ state: "incomplete", review: checked, receipt: null }, id, checked.recipeHash)).receipt, null);
  await assert.rejects(assertDownloadReadback({ state: "incomplete", review: checked, receipt: result }, id, checked.recipeHash));
  await assert.rejects(assertDownloadReadback({ state: "complete", review: checked, receipt: null }, id, checked.recipeHash));
});
for (const field of ["operation", "recipe", "manifest", "manifest-hash", "review-hash", "actual-hash", "actual-size", "flags", "order", "total", "ready"]) test(`receipt refuses forged ${field}`, async () => {
  const value = structuredClone(await receipt());
  if (field === "operation") value.operationId = other;
  else if (field === "recipe") value.recipeHash = hash;
  else if (field === "manifest") value.report.manifest.modelName = "different";
  else if (field === "manifest-hash") value.report.manifestHash = hash;
  else if (field === "review-hash") value.report.reviewHash = hash;
  else if (field === "actual-hash") value.report.files[0].actualHash = "b".repeat(64);
  else if (field === "actual-size") value.report.files[0].actualSize++;
  else if (field === "flags") value.report.files[0].expectedHashChecked = false;
  else if (field === "order") value.report.files.reverse();
  else if (field === "total") value.report.totalBytes++;
  else value.ready = true;
  await assert.rejects(assertDownloadReceipt(value, id, await review()));
});
test("progress ignores wrong operation/hash/size, regressions and malformed values; 100% is only a number", async () => {
  const checked = await review(), progress = { operationId: id, recipeHash: checked.recipeHash, totalBytes: 33, receivedBytes: 33 };
  assert.equal(acceptDownloadProgress(progress, id, checked, 10), 33);
  for (const bad of [null, { ...progress, operationId: other }, { ...progress, recipeHash: hash }, { ...progress, totalBytes: 34 }, { ...progress, receivedBytes: 34 }, { ...progress, receivedBytes: 9 }]) assert.equal(acceptDownloadProgress(bad, id, checked, 10), null);
});
test("catalog is marker metadata only, keeps attention and rejects duplicates or fake support", () => {
  const entry = { operationId: id, recipeHash: hash, modelName: "fixture", modelVersion: "1", totalBytes: 33, state: "completion_recorded" };
  assert.equal(assertDownloadCatalog({ supported: true, unknownEntries: 1, entries: [entry] }).entries[0].state, "completion_recorded");
  assert.equal(assertDownloadCatalog({ supported: true, unknownEntries: 0, entries: [{ ...entry, state: "attention", recipeHash: null, modelName: null, modelVersion: null, totalBytes: null }] }).entries.length, 1);
  for (const bad of [{ supported: false, unknownEntries: 0, entries: [entry] }, { supported: true, unknownEntries: 0, entries: [entry, entry] }, { supported: true, unknownEntries: 0, entries: [{ ...entry, recipeHash: null }] }]) assert.throws(() => assertDownloadCatalog(bad));
});
test("lifetime retains inactive but uncancelled uncertainty and refuses nonboolean fields", () => {
  assert.deepEqual(assertDownloadLifetime({ active: false, cancelled: false }), { active: false, cancelled: false });
  for (const bad of [null, {}, { active: "false", cancelled: true }, { active: false, cancelled: 1 }]) assert.throws(() => assertDownloadLifetime(bad));
});
test("copy readback binds exact receipt; partial/no receipt is not a successful placement", () => {
  const receipt = { packageId: other, reviewHash: hash, totalBytes: 33, placementRecorded: false, engineCompatibility: "not_verified", ready: false };
  assert.equal(assertDownloadCopyResult({ active: false, cancelled: true, receipt }, other, hash, 33).receipt?.placementRecorded, false);
  assert.equal(assertDownloadCopyResult({ active: true, cancelled: true, receipt: null }, other, hash, 33).active, true);
  for (const bad of [{ ...receipt, packageId: id }, { ...receipt, reviewHash: "b".repeat(64) }, { ...receipt, totalBytes: 34 }, { ...receipt, ready: true }]) assert.throws(() => assertDownloadCopyResult({ active: false, cancelled: true, receipt: bad }, other, hash, 33));
});
test("placement catalog source type and origin are mutually bound without changing legacy fixtures", () => {
  const base = { placementRecorded: true, ready: false as const };
  assert.match(placementLabel(base), /配置記録/);
  assert.match(placementLabel({ ...base, sourceType: "download", download: { operationId: id, recipeHash: hash } }), /配置記録/);
  for (const bad of [{ ...base, sourceType: "download" as const }, { ...base, sourceType: "manual_import" as const, download: { operationId: id, recipeHash: hash } }, { ...base, sourceType: "download" as const, download: { operationId: "../x", recipeHash: hash } }]) assert.throws(() => placementLabel(bad));
});
