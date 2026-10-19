import test from "node:test";
import assert from "node:assert/strict";
import { canonicalRecipe, parseDownloadDefinition, downloadDigest, downloadByteTotals, assertDownloadReview, assertOfficialModelOptions, assertDownloadReceipt, assertDownloadReadback, assertDownloadCatalog, assertDownloadLifetime, acceptDownloadProgress, assertDownloadCopyResult, bundledWhisperSource } from "../src/model-download-state.ts";
import { readFileSync } from "node:fs";
import { placementLabel } from "../src/model-inspection-state.ts";
const id = "11111111-1111-4111-8111-111111111111", other = "22222222-2222-4222-8222-222222222222", hash = "a".repeat(64);
const recipe = canonicalRecipe({ schemaVersion: 4, manifest: { schemaVersion: 1, engine: "whisper.cpp", engineVersion: "test", modelName: "合成素材", modelVersion: "1", licenseRef: "fixture only", target: "macos_aarch64", files: [{ relativePath: "cli", role: "executable", expectedSize: 16, expectedHash: hash }, { relativePath: "model", role: "asr", expectedSize: 17, expectedHash: hash }] }, sources: [{ relativePath: "cli", url: "https://example.com/cli" }, { relativePath: "model", url: "https://example.com/model.tar", archive: { format: "tar", expectedSize: 100, expectedHash: hash, memberPath: "model" } }] });
async function review() { return assertDownloadReview({ manifest: recipe.manifest, sources: recipe.sources, recipeSchemaVersion: 4, ...downloadByteTotals(recipe), recipeHash: await downloadDigest(recipe), totalBytes: 33, installed: false, ready: false, publisherAuthenticity: "not_verified" }); }
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
  assert.equal(await downloadDigest(value), "aeffb9a375e04f37c51d6c2c9c3c5f817ef0c6315893005418523a4892c914e2");
});
for (const field of ["schema", "missing-size", "missing-hash", "source-order", "total", "unknown", "byte-limit"]) test(`definition rejects ${field} before review`, () => {
  const value = structuredClone(recipe);
  if (field === "schema") value.schemaVersion = 1 as 4;
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
test("current reviews require explicit schema and transfer/archive byte totals without legacy defaults", async () => {
  for (const field of ["recipeSchemaVersion", "downloadBytes", "archiveBytes"]) {
    const value = await review();
    Reflect.deleteProperty(value, field);
    await assert.rejects(assertDownloadReview(value));
  }
  for (const version of [1, 0, 2, 3, 5, "4", null]) {
    const value = { ...await review(), recipeSchemaVersion: version };
    await assert.rejects(assertDownloadReview(value));
  }
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
  const checked = await review(), progress = { operationId: id, recipeHash: checked.recipeHash, totalBytes: checked.downloadBytes, receivedBytes: checked.downloadBytes };
  assert.equal(acceptDownloadProgress(progress, id, checked, 10), checked.downloadBytes);
  for (const bad of [null, { ...progress, operationId: other }, { ...progress, recipeHash: hash }, { ...progress, totalBytes: checked.downloadBytes + 1 }, { ...progress, receivedBytes: checked.downloadBytes + 1 }, { ...progress, receivedBytes: 9 }]) assert.equal(acceptDownloadProgress(bad, id, checked, 10), null);
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
function archivedRecipe() {
  const value = structuredClone(recipe); value.schemaVersion = 4;
  value.sources = value.sources.map(source => ({ ...source, url: "https://example.com/engine.tar.gz", archive: { format: "tar" as const, expectedSize: 100, expectedHash: hash, memberPath: `package/${source.relativePath}` } }));
  return canonicalRecipe(value);
}
function bundledRecipe() {
  const value = structuredClone(recipe); value.schemaVersion = 4; value.manifest.engineVersion = "v1.9.4";
  value.sources[0] = { ...value.sources[0], url: bundledWhisperSource, archive: { format: "tar", expectedSize: 100, expectedHash: hash, memberPath: "bin/whisper-cli" } };
  value.sources[1] = { relativePath: "model", url: "https://example.com/model" };
  return canonicalRecipe(value);
}
test("hybrid bundle recipe separates wire traffic and storage; strict review and receipt survive readback", async () => {
  const value = bundledRecipe(), totals = downloadByteTotals(value);
  assert.deepEqual(totals, { downloadBytes: 17, archiveBytes: 100, bundledBytes: 100 });
  const native = { manifest: value.manifest, sources: value.sources, recipeSchemaVersion: 4, ...totals, recipeHash: await downloadDigest(value), totalBytes: 33, installed: false, ready: false, publisherAuthenticity: "not_verified" };
  const checked = await assertDownloadReview(native);
  assert.equal(acceptDownloadProgress({ operationId: id, recipeHash: checked.recipeHash, receivedBytes: 17, totalBytes: 17 }, id, checked, 0), 17);
  for (const patch of [{ bundledBytes: 0 }, { downloadBytes: 117 }, { archiveBytes: 0 }, { recipeSchemaVersion: 2 }, { recipeSchemaVersion: "3" }, { ready: true }]) await assert.rejects(assertDownloadReview({ ...native, ...patch }));
  const body = await receipt(); body.recipeHash = checked.recipeHash; body.report.manifest = value.manifest;
  body.report.manifestHash = await downloadDigest(value.manifest);
  body.report.reviewHash = await downloadDigest([1, value.manifest, body.report.manifestHash, body.report.files]);
  assert.equal((await assertDownloadReadback({ state: "complete", review: checked, receipt: body }, id, checked.recipeHash)).state, "complete");
  assert.equal((await assertDownloadReadback({ state: "incomplete", review: checked, receipt: null }, id, checked.recipeHash)).receipt, null);
});
test("bundled source is a fixed native ID, not legacy network or arbitrary local input", () => {
  for (const variant of ["schema1", "schema2", "id", "member", "raw", "engine", "version", "target", "network-asr"]) {
    const value = bundledRecipe();
    if (variant === "schema1") value.schemaVersion = 1 as 4;
    else if (variant === "schema2") Object.assign(value, {schemaVersion:2});
    else if (variant === "id") value.sources[0].url = "mon-bundled://other/runtime.tar";
    else if (variant === "member") value.sources[0].archive!.memberPath = "bin/other";
    else if (variant === "raw") delete value.sources[0].archive;
    else if (variant === "engine") value.manifest.engine = "llama.cpp";
    else if (variant === "version") value.manifest.engineVersion = "v0";
    else if (variant === "target") value.manifest.target = "windows_x86_64";
    else value.sources[1].url = bundledWhisperSource;
    assert.throws(() => canonicalRecipe(value), variant);
  }
});
test("archive transfer is deduplicated, while installed payload and archive storage are separate", async () => {
  const value = archivedRecipe();
  assert.deepEqual(downloadByteTotals(value), { downloadBytes: 100, archiveBytes: 100 });
  const native = { manifest: value.manifest, sources: value.sources, recipeSchemaVersion: 4, downloadBytes: 100, archiveBytes: 100, recipeHash: await downloadDigest(value), totalBytes: 33, installed: false, ready: false, publisherAuthenticity: "not_verified" };
  const checked = await assertDownloadReview(native);
  assert.equal(acceptDownloadProgress({ operationId: id, recipeHash: checked.recipeHash, receivedBytes: 100, totalBytes: 100 }, id, checked, 0), 100);
  assert.equal(acceptDownloadProgress({ operationId: id, recipeHash: checked.recipeHash, receivedBytes: 33, totalBytes: 33 }, id, checked, 0), null);
  for (const patch of [{ recipeSchemaVersion: 1 }, { downloadBytes: 200 }, { archiveBytes: 0 }, { totalBytes: 100 }, { recipeSchemaVersion: undefined }]) await assert.rejects(assertDownloadReview({ ...native, ...patch }));
  const body = await receipt(); body.recipeHash = checked.recipeHash;
  assert.equal((await assertDownloadReceipt(body, id, checked)).ready, false);
});
test("archive definition rejects duplicate members, mixed use, inconsistent metadata and invalid paths", () => {
  for (const variant of ["version", "duplicate", "raw", "size", "sha", "path", "format", "extra", "zero"]) {
    const value = archivedRecipe();
    if (variant === "version") value.schemaVersion = 1 as 4;
    else if (variant === "duplicate") value.sources[1].archive!.memberPath = value.sources[0].archive!.memberPath;
    else if (variant === "raw") delete value.sources[1].archive;
    else if (variant === "size") value.sources[1].archive!.expectedSize++;
    else if (variant === "sha") value.sources[1].archive!.expectedHash = "b".repeat(64);
    else if (variant === "path") value.sources[0].archive!.memberPath = "../outside";
    else if (variant === "format") Object.assign(value.sources[0].archive!, { format: "zip" });
    else if (variant === "extra") Object.assign(value.sources[0].archive!, { execute: true });
    else value.sources[0].archive!.expectedSize = 0;
    assert.throws(() => canonicalRecipe(value), variant);
  }
});
test("built-in choices bind fixed recipes and archives without downloaded or ready claims", async () => {
  const choices = JSON.parse(readFileSync(new URL("../model-catalog/macos.json", import.meta.url), "utf8"));
  const native = await Promise.all(choices.map(async (choice: any) => {
    const recipe = canonicalRecipe(choice.recipe);
    return { id: choice.id, label: choice.label, description: choice.description, tokenLimits: choice.tokenLimits ?? null, definition: JSON.stringify(recipe), review: {
      manifest: recipe.manifest, sources: recipe.sources, recipeSchemaVersion: 4, ...downloadByteTotals(recipe), recipeHash: await downloadDigest(recipe),
      totalBytes: recipe.manifest.files.reduce((n, f) => n + f.expectedSize, 0), publisherAuthenticity: "not_verified", installed: false, ready: false,
    } };
  }));
  const options = await assertOfficialModelOptions(native);
  assert.equal(options.length, 6);
  assert.equal(options[0].review.manifest.engine, "llama.cpp");
  assert.deepEqual(options[0].tokenLimits, { maxInputTokens: 32768, maxOutputTokens: 8192, maxContextTokens: 32768, tokenizer: "utf8_byte_bpe" });
  assert.equal(options[1].tokenLimits, null);
  assert.equal(options[0].review.manifest.files.filter(f => f.role === "notice").length, 2);
  assert.equal(options[0].review.downloadBytes, 1_129_407_950);
  assert.equal(options[0].review.recipeHash, "4f2b8fe8ba750b9dcf5feb531f13f1929845556b91265d09f51a52e4d215d7a3");
  assert.equal(options[1].review.manifest.engine, "sherpa-onnx");
  assert.equal(options[1].review.manifest.files.filter(f => f.role === "notice").length, 5);
  assert.equal(options[1].review.downloadBytes, 65_153_215);
  assert.equal(options[1].review.archiveBytes, 25_210_612);
  assert.equal(options[1].review.totalBytes, 78_117_057);
  assert.equal(options[1].review.recipeHash, "b564f0b5f9b2b88dbfebed9cb7c5ff6f12403b7ee5619701aa582153d443e666");
  assert.deepEqual(options.slice(2, 4).map(o => o.review.recipeHash), [
    "a8f5330b3cd2a0c700b81b0d0402473c441dad3faa3814060162ec132d1fdd59",
    "e1dc02a6b2a5224f76e2432f7daaf97154427fccb3c16627340ccf07380c8b96",
  ]);
  for (const option of options.filter(o => o.review.manifest.engine === "llama.cpp" && !o.id.includes("qwen3-"))) {
    assert.deepEqual(option.tokenLimits, options[0].tokenLimits);
    assert.match(option.label, /Qwen.*Q[48]/);
  }
  assert.equal(options[3].tokenLimits, null);
  const qwen3 = options[4];
  assert.equal(qwen3.label, "Qwen 3 8B · Q4_K_M");
  assert.deepEqual(qwen3.tokenLimits, { maxInputTokens: 40960, maxOutputTokens: 32768, maxContextTokens: 40960, tokenizer: "utf8_byte_bpe" });
  assert.equal(qwen3.review.recipeHash, "344413ffe456e123c918af5f422a82f64f7443fa6a81f332a58068caeed12f14");
  assert.equal(qwen3.review.manifest.files.find(file => file.role === "llm")?.expectedHash, "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785");
  const qwen3_4b = options.find(o => o.id === "llama-b11533-qwen3-4b-q4km")!;
  assert.equal(qwen3_4b.label, "Qwen 3 4B · Q4_K_M");
  assert.deepEqual(qwen3_4b.tokenLimits, qwen3.tokenLimits);
  assert.equal(qwen3_4b.review.recipeHash, "920cc12b71fdc8d1dc66234050be25d0c577f16a639e4bf55048ca8e634e70b8");
  const model4b = qwen3_4b.review.manifest.files.find(file => file.role === "llm")!;
  assert.equal(model4b.expectedHash, "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5");
  assert.equal(model4b.expectedSize, 2_497_280_256);
  assert.equal(options[3].review.manifest.files.find(f => f.role === "segmentation")?.expectedHash,
    "d582f4b4c6b48205de7e0643c57df0df5615a3c176189be3fc461e9d18827b5d");
  await assert.rejects(assertOfficialModelOptions([native[0], native[0]]));
  await assert.rejects(assertOfficialModelOptions([{ ...native[0], definition: JSON.stringify(recipe) }]));
  await assert.rejects(assertOfficialModelOptions([{ ...native[0], review: { ...native[0].review, ready: true } }]));
  for (const patch of [null, { maxInputTokens: 0 }, { maxOutputTokens: 32769 }, { maxContextTokens: 0 },
    { maxInputTokens: 32769 }, { maxOutputTokens: 1.5 }, { maxInputTokens: "32768" }, { tokenizer: "unknown" }, { extra: 1 }]) {
    const tokenLimits = patch === null ? null : { ...native[0].tokenLimits, ...patch };
    await assert.rejects(assertOfficialModelOptions([{ ...native[0], tokenLimits }]));
  }
  await assert.rejects(assertOfficialModelOptions([{ ...native[1], tokenLimits: native[0].tokenLimits }]));
});
