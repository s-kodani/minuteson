import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import {
  canonicalRecipe, downloadByteTotals, downloadDigest, assertDownloadReview,
  type DownloadRecipe,
} from "../src/model-download-state.ts";
import licensePolicy from "../model-catalog/model-license-policy.json" with { type: "json" };

const json = (path: string) => JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8"));
const definitions = json("../model-catalog/model-definitions.json");
const networks = [json("../model-catalog/whisper-network.json"), json("../model-catalog/whisper-vad-network.json")];
const onnxNotice = licensePolicy.notices.find(notice => notice.name === "onnxruntime-MIT.txt")!;

function noticeRecipe(target = "macos_aarch64"): DownloadRecipe {
  return canonicalRecipe({ schemaVersion: 4, manifest: {
    schemaVersion: 1, engine: "whisper.cpp", engineVersion: "fixture", modelName: "offline notice",
    modelVersion: "1", target, licenseRef: "fixture", files: [{
      relativePath: "notices/onnxruntime-MIT.txt", role: "notice",
      expectedSize: onnxNotice.bytes, expectedHash: onnxNotice.sha256,
    }],
  }, sources: [{ relativePath: "notices/onnxruntime-MIT.txt", url: onnxNotice.sourceUrl }] });
}

test("all official standalone notices have exact embedded bytes and require no transfer", () => {
  const unique = new Set<string>();
  let references = 0;
  for (const row of [...definitions, ...networks]) {
    row.files.forEach((file: any, index: number) => {
      const source = row.sources[index];
      if (file.role !== "notice" || source.archive) return;
      const notice = licensePolicy.notices.find(notice => notice.sourceUrl === source.url
        && notice.bytes === file.expectedSize && notice.sha256 === file.expectedHash);
      assert.ok(notice, `${row.id ?? "Whisper"}: ${file.relativePath} must be embedded`);
      const bytes = readFileSync(new URL(`../../third-party/model-licenses/${notice.file}`, import.meta.url));
      assert.equal(bytes.length, file.expectedSize);
      assert.equal(createHash("sha256").update(bytes).digest("hex"), file.expectedHash);
      for (const target of ["macos_aarch64", "windows_x86_64"]) {
        const recipe = noticeRecipe(target);
        recipe.manifest.files = [file]; recipe.sources = [source];
        assert.deepEqual(downloadByteTotals(recipe), { downloadBytes: 0, archiveBytes: 0 });
      }
      unique.add(JSON.stringify([source.url, file.expectedSize, file.expectedHash]));
      references++;
    });
  }
  assert.equal(unique.size, 14);
  assert.equal(references, 27);
});

test("ReazonSpeech still counts weights and its runtime archive while excluding all five standalone notices", () => {
  for (const option of json("../model-catalog/asr-transducer.json")) {
    const recipe = canonicalRecipe(option.recipe);
    const modelBytes = recipe.manifest.files.filter(file => ["asr", "asr_asset", "vad"].includes(file.role))
      .reduce((total, file) => total + file.expectedSize, 0);
    const archiveBytes = 18_252_168;
    assert.deepEqual(downloadByteTotals(recipe), { downloadBytes: modelBytes + archiveBytes, archiveBytes });
    assert.equal(recipe.manifest.files.filter(file => file.role === "notice").length, 5);
  }
});

test("only an exact standalone notice match can reduce the transfer estimate", () => {
  for (const change of ["url", "size", "hash", "role", "path"]) {
    const recipe = noticeRecipe();
    if (change === "url") recipe.sources[0].url = "https://example.com/other-notice";
    if (change === "size") recipe.manifest.files[0].expectedSize++;
    if (change === "hash") recipe.manifest.files[0].expectedHash = "a".repeat(64);
    if (change === "role") recipe.manifest.files[0].role = "runtime_library";
    if (change === "path") recipe.sources[0].relativePath = "notices/other.txt";
    assert.equal(downloadByteTotals(recipe).downloadBytes, recipe.manifest.files[0].expectedSize, change);
  }
  const archived = noticeRecipe();
  archived.sources[0].archive = { format: "tar", expectedSize: 4096, expectedHash: "b".repeat(64), memberPath: "LICENSE" };
  assert.deepEqual(downloadByteTotals(archived), { downloadBytes: 4096, archiveBytes: 4096 });
});

test("excluding local notice traffic never excludes its bytes from the package capacity limit", () => {
  const recipe = noticeRecipe();
  recipe.manifest.files.push({ relativePath: "model.bin", role: "asr", expectedSize: 32 * 1024 ** 3, expectedHash: "a".repeat(64) });
  recipe.sources.push({ relativePath: "model.bin", url: "https://example.com/model.bin" });
  assert.equal(downloadByteTotals(recipe).downloadBytes, 32 * 1024 ** 3);
  assert.throws(() => canonicalRecipe(recipe), "the local notice still occupies package storage");
});

test("native review readers reject stale transfer totals without changing recipe identity", async () => {
  for (const target of ["macos_aarch64", "windows_x86_64"]) {
    const recipe = noticeRecipe(target), recipeHash = await downloadDigest(recipe);
    const native = { manifest: recipe.manifest, sources: recipe.sources, recipeSchemaVersion: 4,
      downloadBytes: 0, archiveBytes: 0, recipeHash, totalBytes: onnxNotice.bytes,
      publisherAuthenticity: "not_verified", installed: false, ready: false };
    assert.equal((await assertDownloadReview(native)).recipeHash, recipeHash);
    await assert.rejects(assertDownloadReview({ ...native, downloadBytes: onnxNotice.bytes }));
    assert.equal(await downloadDigest(recipe), recipeHash);
  }
});
