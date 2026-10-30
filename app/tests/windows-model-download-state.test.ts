import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  canonicalRecipe, downloadByteTotals, downloadDigest, assertDownloadReview,
  assertOfficialModelOptions, bundledWindowsSource,
} from "../src/model-download-state.ts";

const sha = "a".repeat(64);
function windowsRecipe(engine = "whisper.cpp") {
  return {
    schemaVersion: 4,
    manifest: { schemaVersion: 1, engine, engineVersion: "fixture", modelName: "Windows fixture", modelVersion: "1",
      quantization: null, licenseRef: "fixture only", target: "windows_x86_64",
      files: [
        { relativePath: "engine.exe", role: "executable", expectedSize: 10, expectedHash: sha },
        { relativePath: "engine.dll", role: "runtime_library", expectedSize: 20, expectedHash: sha },
        { relativePath: "model.bin", role: engine === "llama.cpp" ? "llm" : "asr", expectedSize: 30, expectedHash: sha },
      ] },
    sources: [
      { relativePath: "engine.exe", url: bundledWindowsSource, archive: { format: "tar", expectedSize: 100, expectedHash: sha, memberPath: "engine/engine.exe" } },
      { relativePath: "engine.dll", url: bundledWindowsSource, archive: { format: "tar", expectedSize: 100, expectedHash: sha, memberPath: "engine/engine.dll" } },
      { relativePath: "model.bin", url: "https://example.com/model.bin" },
    ],
  };
}

test("Windows bundle definitions retain native recipe digests and count the archive once without network bytes", async () => {
  for (const engine of ["whisper.cpp", "sherpa-onnx", "llama.cpp"]) {
    const recipe = canonicalRecipe(windowsRecipe(engine));
    assert.deepEqual(downloadByteTotals(recipe), { downloadBytes: 30, archiveBytes: 100, bundledBytes: 100 });
    const review = { ...recipe, recipeSchemaVersion: 4, ...downloadByteTotals(recipe),
      recipeHash: await downloadDigest(recipe), totalBytes: 60, publisherAuthenticity: "not_verified", installed: false, ready: false };
    delete (review as Record<string, unknown>).schemaVersion;
    const option = { id: "windows-fixture", label: "Windows CPU", description: "fixture", definition: JSON.stringify(recipe), review,
      tokenLimits: engine === "llama.cpp" ? { maxInputTokens: 2048, maxOutputTokens: 512, maxContextTokens: 4096, tokenizer: "utf8_byte_bpe" } : null };
    assert.equal((await assertOfficialModelOptions([option])).length, 1);
    await assert.rejects(assertDownloadReview({ ...review, bundledBytes: 0 }));
    await assert.rejects(assertDownloadReview({ ...review, downloadBytes: 130 }));
    await assert.rejects(assertDownloadReview({ ...review, recipeHash: "b".repeat(64) }));
  }
});

test("Windows bundled files still require the fixed URI, target, archive, safe member and native role", () => {
  for (const change of ["uri", "target", "raw", "member", "role", "hash"]) {
    const recipe = windowsRecipe();
    if (change === "uri") recipe.sources[0].url += "-other";
    if (change === "target") recipe.manifest.target = "macos_aarch64";
    if (change === "raw") delete recipe.sources[0].archive;
    if (change === "member") recipe.sources[0].archive!.memberPath = "../escape";
    if (change === "role") recipe.manifest.files[0].role = "asr";
    if (change === "hash") recipe.sources[0].archive!.expectedHash = "bad";
    assert.throws(() => canonicalRecipe(recipe), change);
  }
});

test("all Windows model choices from the native IPC pass the production frontend reader", {
  skip: !process.env.MON_WINDOWS_MODEL_OPTIONS && "requires the native Windows IPC fixture",
}, async () => {
  const native = JSON.parse(readFileSync(process.env.MON_WINDOWS_MODEL_OPTIONS!, "utf8"));
  const choices = await assertOfficialModelOptions(native);
  assert.equal(choices.length, 19);
  assert.deepEqual(choices.map(o => o.review.manifest.engine).reduce((counts, engine) => {
    counts[engine] = (counts[engine] ?? 0) + 1; return counts;
  }, {} as Record<string, number>), { "whisper.cpp": 6, "sherpa-onnx": 2, "llama.cpp": 9, "asr": 2 });
  for (const model of ["gemma4-e4b-it", "phi4-mini-instruct", "lfm3-17b"]) {
    const choice = choices.find(option => option.id === `windows-llama-b11960-${model}-q4km`);
    assert.ok(choice, `missing Windows model: ${model}`);
    assert.equal(choice.review.manifest.engineVersion, "b11960");
    assert.equal(choice.tokenLimits?.tokenizer, "gguf");
  }
  for (const choice of choices) assert.ok(choice.review.bundledBytes! > 0);
});
