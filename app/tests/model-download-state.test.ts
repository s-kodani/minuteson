import { modelLicenseSummaryFixture } from "./model-license-fixture.ts";
import test from "node:test";
import assert from "node:assert/strict";
import { canonicalRecipe, parseDownloadDefinition, downloadDigest, downloadByteTotals, assertDownloadReview, assertOfficialModelOptions, assertDownloadReceipt, assertDownloadReadback, assertDownloadCatalog, assertDownloadLifetime, assertDownloadCopyResult, bundledWhisperSource, bundledAsrSource, bundledWindowsSource } from "../src/model-download-state.ts";
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

test("ReazonSpeech model assets are separate from the bundled runtime on either platform", () => {
  const definitions = JSON.parse(readFileSync(new URL("../model-catalog/asr-transducer.json", import.meta.url), "utf8"));
  const reazon = definitions.find((option: any) => option.id === "reazonspeech-k2-v2-fp32").recipe;
  assert.equal(canonicalRecipe(reazon).manifest.backend?.id, "sherpa-onnx-transducer");
  for (const [target, url] of [["macos_aarch64", bundledAsrSource], ["windows_x86_64", bundledWindowsSource]]) {
    const value = structuredClone(reazon);
    value.manifest.target = target;
    const asset = value.sources.find((source: any) => source.relativePath === value.manifest.backend.files.tokens);
    asset.url = url;
    asset.archive = { format: "tar", expectedSize: 10240, expectedHash: hash, memberPath: "models/tokens.txt" };
    assert.throws(() => canonicalRecipe(value));
  }
});

test("package digest matches native receipt proof and ignores archive transport identity", async () => {
  const first = await review(), result = await receipt();
  assert.equal(first.packageReviewHash, result.report.reviewHash);
  const changed = structuredClone(recipe); changed.sources[1].archive!.expectedHash = "c".repeat(64);
  const second = await assertDownloadReview({ ...first, ...changed, recipeHash: await downloadDigest(changed) });
  assert.notEqual(first.recipeHash, second.recipeHash); assert.equal(first.packageReviewHash, second.packageReviewHash);
  const untrusted = await assertDownloadReview({ ...first, packageReviewHash: "f".repeat(64) });
  assert.equal(untrusted.packageReviewHash, first.packageReviewHash, "derive package identity instead of trusting a wire field");
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
test("catalog is marker metadata only, keeps attention and rejects duplicates or fake support", () => {
  const entry = { operationId: id, recipeHash: hash, modelName: "fixture", modelVersion: "1", totalBytes: 33, state: "completion_recorded" };
  assert.equal(assertDownloadCatalog({ supported: true, unknownEntries: 1, entries: [entry] }).entries[0].state, "completion_recorded");
  for (const state of ["pending", "committed"]) {
    assert.equal(assertDownloadCatalog({ supported: true, unknownEntries: 0, entries: [{ ...entry, placement: { packageId: other, state } }] }).entries[0].placement?.state, state);
  }
  for (const placement of [{packageId:"../outside",state:"pending"},{packageId:other,state:"ready"},{packageId:other,state:"pending",directory:"/outside"}]) {
    assert.throws(() => assertDownloadCatalog({ supported: true, unknownEntries: 0, entries: [{ ...entry, placement }] }));
  }
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
    return { licenseSummary: modelLicenseSummaryFixture, id: choice.id, label: choice.label, description: choice.description, tokenLimits: choice.tokenLimits ?? null, definition: JSON.stringify(recipe), review: {
      manifest: recipe.manifest, sources: recipe.sources, recipeSchemaVersion: 4, ...downloadByteTotals(recipe), recipeHash: await downloadDigest(recipe),
      totalBytes: recipe.manifest.files.reduce((n, f) => n + f.expectedSize, 0), publisherAuthenticity: "not_verified", installed: false, ready: false,
    } };
  }));
  const options = await assertOfficialModelOptions(native);
  assert.equal(options.length, 7);
  const extraChoices = [...native, ...native.map((option, i) => ({ ...option, id: `extra-${i}` }))];
  assert.equal((await assertOfficialModelOptions(extraChoices)).length, 14);
  await assert.rejects(assertOfficialModelOptions(Array.from({ length: 33 }, (_, i) => ({ ...native[0], id: `extra-${i}` }))));
  const gemma = options.find(o => o.id === "llama-b11960-gemma4-e4b-it-q4km")!;
  assert.equal(gemma.label, "Gemma 4 E4B IT · Q4_K_M");
  assert.deepEqual(gemma.tokenLimits, { maxInputTokens: 131072, maxOutputTokens: 32768, maxContextTokens: 131072, tokenizer: "gguf" });
  assert.equal(gemma.review.manifest.engineVersion, "b11960");
  assert.equal(gemma.review.manifest.quantization, "Q4_K_M");
  assert.equal(gemma.review.manifest.files.filter(f => f.role === "notice").length, 2);
  assert.equal(gemma.review.ready, false);

  const sherpa = options.find(o => o.id === "sherpa-1138-pyannote3-eres2net-base")!;
  assert.equal(sherpa.tokenLimits, null);
  assert.equal(sherpa.review.manifest.files.filter(f => f.role === "notice").length, 5);
  assert.equal(sherpa.review.downloadBytes, 65_153_215);
  assert.equal(sherpa.review.archiveBytes, 25_210_612);
  assert.equal(sherpa.review.totalBytes, 78_117_057);
  assert.equal(sherpa.review.recipeHash, "b564f0b5f9b2b88dbfebed9cb7c5ff6f12403b7ee5619701aa582153d443e666");
  const int8 = options.find(o => o.id === "sherpa-1138-pyannote3-int8-eres2net-base")!;
  assert.equal(int8.tokenLimits, null);
  assert.equal(int8.review.recipeHash, "e1dc02a6b2a5224f76e2432f7daaf97154427fccb3c16627340ccf07380c8b96");
  const qwen3 = options.find(o => o.id === "llama-b11533-qwen3-8b-q4km")!;
  assert.equal(qwen3.label, "Qwen 3 8B · Q4_K_M");
  assert.deepEqual(qwen3.tokenLimits, { maxInputTokens: 40960, maxOutputTokens: 32768, maxContextTokens: 40960, tokenizer: "utf8_byte_bpe" });
  assert.equal(qwen3.review.recipeHash, "d985b93ece8e0c10e87af2a2a7342be13f22a4bba729c7f8ffb44fd306b597ee");
  assert.equal(qwen3.review.manifest.files.find(file => file.role === "llm")?.expectedHash, "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785");
  const qwen3_4b = options.find(o => o.id === "llama-b11533-qwen3-4b-q4km")!;
  assert.equal(qwen3_4b.label, "Qwen 3 4B · Q4_K_M");
  assert.deepEqual(qwen3_4b.tokenLimits, qwen3.tokenLimits);
  assert.equal(qwen3_4b.review.recipeHash, "19548a8e0a3221266c8d231c2146f85f6575c3cb71adadf89de51b000eb29a66");
  const model4b = qwen3_4b.review.manifest.files.find(file => file.role === "llm")!;
  assert.equal(model4b.expectedHash, "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5");
  assert.equal(model4b.expectedSize, 2_497_280_256);
  for (const [size, bytes, hash, recipeHash] of [
    ["4B", 2_740_937_888, "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4", "0cef3f0c5a4ff344b1c52b1b0e793a6f0dad8c85528c79dad11c9d3fbfd30d82"],
    ["9B", 5_680_522_464, "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8", "9ff8c744fd3f2d8094d28a9830b0b5b829fa4f5944441b92c21a471af54327a2"],
  ] as const) {
    const option = options.find(o => o.id === `llama-b11533-qwen35-${size.toLowerCase()}-q4km`)!;
    assert.equal(option.label, `Qwen 3.5 ${size} · Q4_K_M`);
    assert.deepEqual(option.tokenLimits, { maxInputTokens: 262144, maxOutputTokens: 32768, maxContextTokens: 262144, tokenizer: "utf8_byte_bpe" });
    assert.equal(option.review.recipeHash, recipeHash);
    const file = option.review.manifest.files.find(f => f.role === "llm")!;
    assert.equal(file.expectedHash, hash);
    assert.equal(file.expectedSize, bytes);
    const source = option.review.sources.find(f => f.relativePath === file.relativePath)!;
    assert.match(source.url!, new RegExp(`^https://huggingface.co/unsloth/Qwen3\\.5-${size}-GGUF/resolve/[0-9a-f]{40}/`));
  }
  assert.equal(int8.review.manifest.files.find(f => f.role === "segmentation")?.expectedHash,
    "d582f4b4c6b48205de7e0643c57df0df5615a3c176189be3fc461e9d18827b5d");
  const qwenNative = native.find(option => option.id === qwen3_4b.id);
  const sherpaNative = native.find(option => option.id === sherpa.id);
  await assert.rejects(assertOfficialModelOptions([qwenNative, qwenNative]));
  await assert.rejects(assertOfficialModelOptions([{ ...qwenNative, definition: JSON.stringify(recipe) }]));
  await assert.rejects(assertOfficialModelOptions([{ ...qwenNative, review: { ...qwenNative.review, ready: true } }]));
  for (const patch of [null, { maxInputTokens: 0 }, { maxOutputTokens: 40961 }, { maxContextTokens: 0 },
    { maxInputTokens: 40961 }, { maxOutputTokens: 1.5 }, { maxInputTokens: "40960" }, { tokenizer: "unknown" }, { extra: 1 }]) {
    const tokenLimits = patch === null ? null : { ...qwenNative.tokenLimits, ...patch };
    await assert.rejects(assertOfficialModelOptions([{ ...qwenNative, tokenLimits }]));
  }
  await assert.rejects(assertOfficialModelOptions([{ ...sherpaNative, tokenLimits: qwenNative.tokenLimits }]));
});
