import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { canonicalRecipe, downloadByteTotals, downloadDigest, type OfficialModelOption } from "../src/model-download-state.ts";
import { canReuseOfficialModel, installedModelGroups, matchesOfficialModel, modelChoices, modelChoiceAction, modelDisplayName } from "../src/model-selection-state.ts";
import type { ModelPlacementEntry } from "../src/model-inspection-state.ts";
import type { ModelProfileView } from "../src/model-profile-state.ts";

const catalog = JSON.parse(readFileSync(new URL("../model-catalog/macos.json", import.meta.url), "utf8"));
const options: OfficialModelOption[] = await Promise.all(catalog.map(async (choice: any) => {
  const recipe = canonicalRecipe(choice.recipe);
  return { ...choice, definition: JSON.stringify(recipe), review: { ...recipe, recipeSchemaVersion: recipe.schemaVersion,
    recipeHash: await downloadDigest(recipe), ...downloadByteTotals(recipe), totalBytes: recipe.manifest.files.reduce((n, f) => n + f.expectedSize, 0),
    installed: false, ready: false, publisherAuthenticity: "not_verified" } };
}));
function entry(option = options[0], packageId = "installed"): ModelPlacementEntry {
  const manifest = option.review.manifest;
  return { packageId, engine: manifest.engine, engineVersion: manifest.engineVersion, modelName: manifest.modelName,
    modelVersion: manifest.modelVersion, quantization: manifest.quantization, totalBytes: option.review.totalBytes,
    placementRecorded: true, ready: false, sourceType: "download", reviewHash: "a".repeat(64),
    download: { operationId: "33333333-3333-4333-8333-333333333333", recipeHash: option.review.recipeHash },
    modelFiles: manifest.files.filter(file => !["executable", "runtime_library", "notice"].includes(file.role))
      .map(file => ({ role: file.role, size: file.expectedSize, sha256: file.expectedHash })) };
}
function profile(packageId: string): ModelProfileView[] {
  return [{ name: "standard", id: "profile", profileVersion: 1, configurationComplete: false,
    components: [{ roles: ["llm"], packageId, modelName: "old stored name", modelVersion: "version", engine: "llama.cpp",
      checkOperationId: "check", settingsMatch: true, compatibilityCurrent: true }] }];
}
test("downloaded choice retains its value and exact label without a second installed option", () => {
  const before = modelChoices("llama.cpp", [], options, []);
  const installed = entry();
  const after = modelChoices("llama.cpp", [installed], options, profile(installed.packageId));
  assert.deepEqual(after.map(c => [c.value, c.label]), before.map(c => [c.value, c.label]));
  assert.equal(after[0].entry, installed);
  assert.equal(modelDisplayName(installed, options), options[0].label);
  assert.equal(installed.modelName, "Qwen2.5-1.5B-Instruct");
});
test("older download recipes reuse exact model bytes and display the catalog name", () => {
  const installed = entry(); installed.download!.recipeHash = "b".repeat(64);
  installed.engineVersion = "older-runtime";
  assert.equal(matchesOfficialModel(installed, options[0]), true);
  assert.equal(modelChoices("llama.cpp", [installed], options, [])[0].entry, installed);
  assert.equal(modelDisplayName(installed, options), options[0].label);
});
test("Whisper keeps its model name but prepares the current engine instead of reusing a CPU-only recipe", () => {
  const option = structuredClone(options[0]);
  option.review.manifest.engine = "whisper.cpp";
  for (const file of option.review.manifest.files) if (file.role === "llm") file.role = "asr";
  const current = entry(option, "metal"), old = entry(option, "cpu");
  old.download!.recipeHash = "b".repeat(64);
  assert.equal(matchesOfficialModel(old, option), true);
  assert.equal(canReuseOfficialModel(old, option), false);
  assert.equal(canReuseOfficialModel(current, option), true);
  const oldOnly = modelChoices("whisper.cpp", [old], [option], []);
  assert.equal(oldOnly.length, 1);
  assert.equal(oldOnly[0].entry, undefined);
  assert.deepEqual(oldOnly[0].packageIds, [old.packageId]);
  const upgraded = modelChoices("whisper.cpp", [old, current], [option], []);
  assert.equal(upgraded[0].entry, current);
  assert.equal(modelDisplayName(old, [option]), option.label);
});
test("duplicate packages collapse and preserve the package currently selected by the profile", () => {
  const current = entry(options[0], "current"), newer = entry(options[0], "newer");
  const choices = modelChoices("llama.cpp", [newer, current], options, profile("current"));
  assert.equal(choices.length, 9);
  assert.equal(choices[0].entry?.packageId, "current");
  assert.deepEqual(choices[0].packageIds, ["newer", "current"]);
  assert.equal(installedModelGroups([newer, current], profile("current")).length, 1);
  assert.equal(installedModelGroups([newer, current], profile("current"))[0].entry.packageId, "current");
});
test("all model families, sizes and quantizations remain independent even if stored names are identical", () => {
  const llms = options.filter(o => o.review.manifest.engine === "llama.cpp");
  const installed = llms.map(o => entry(o, o.id));
  installed.forEach(item => { item.modelName = "same-name"; });
  const choices = modelChoices("llama.cpp", installed, options, []);
  assert.deepEqual(choices.map(c => c.entry?.packageId), llms.map(o => o.id));
  assert.deepEqual(choices.map(c => c.label), llms.map(o => o.label));
});
test("removed and unfinished packages do not hide the official download choice", () => {
  for (const patch of [{ bodyRemoved: true }, { placementRecorded: false }]) {
    const choices = modelChoices("llama.cpp", [{ ...entry(), ...patch }], options, []);
    assert.equal(choices[0].entry, undefined);
    assert.equal(choices.length, 9);
  }
});
test("matching bytes never make an unchecked manual executable reusable", () => {
  const manual = { ...entry(), sourceType: "manual_import" as const, download: null };
  const choices = modelChoices("llama.cpp", [manual], options, []);
  assert.equal(choices.length, 9);
  assert.equal(choices[0].entry, undefined);
});
test("unknown imported model keeps its own name and quantization", () => {
  const other = { ...entry(), modelFiles: [{ role: "llm", size: 10, sha256: "f".repeat(64) }], modelName: "Local model", download: null, sourceType: "manual_import" as const };
  assert.equal(modelDisplayName(other, options), "Local model · Q4_K_M");
  assert.equal(modelChoices("llama.cpp", [other], options, []).length, 10);
});

test("Sherpa keeps model identity but reacquires the package with the voice C API", () => {
  const option = options.find(o => o.review.manifest.engine === "sherpa-onnx")!;
  const current = entry(option, "with-c-api"), old = entry(option, "without-c-api");
  old.download!.recipeHash = "0996bb77a0fab2f604c97af2bde017cebe813e83529228be92778c4befd4a7c8";
  assert.equal(matchesOfficialModel(old, option), true);
  assert.equal(canReuseOfficialModel(old, option), false);
  assert.equal(canReuseOfficialModel(current, option), true);
  const olderOnly = modelChoices("sherpa-onnx", [old], [option], profile(old.packageId));
  assert.equal(olderOnly[0].entry, undefined);
  assert.deepEqual(olderOnly[0].packageIds, [old.packageId]);
  assert.equal(modelChoices("sherpa-onnx", [old, current], [option], [])[0].entry, current);
  for (const choice of options.filter(o => o.review.manifest.engine === "sherpa-onnx")) {
    const runtimes = choice.review.manifest.files.filter(f => f.role === "runtime_library");
    assert.equal(runtimes.length, 2);
    assert.deepEqual(runtimes.find(f => f.relativePath === "lib/libsherpa-onnx-c-api.dylib"), {
      relativePath:"lib/libsherpa-onnx-c-api.dylib",role:"runtime_library",expectedSize:2_999_920,
      expectedHash:"ae729669633008692c670d4e4c942db7ea5bdc38abc2f479516062d48849e9d1",
    });
  }
});

function checkedSelection(option: OfficialModelOption) {
  const installed = entry(option, "11111111-1111-4111-8111-111111111111");
  const operationId = "22222222-2222-4222-8222-222222222222";
  installed.download!.recipeHash = "b".repeat(64);
  installed.compatibility = { operationId, packageId: installed.packageId, reviewHash: installed.reviewHash,
    adapterRevision: 1, state: "succeeded", errorMessage: null };
  const used = { roles: ["asr"], packageId: installed.packageId, modelName: installed.modelName, modelVersion: installed.modelVersion,
    engine: installed.engine as "whisper.cpp" | "sherpa-onnx" | "llama.cpp", checkOperationId: operationId, settingsMatch: true, compatibilityCurrent: true };
  const profiles: ModelProfileView[] = [{ name: "standard", id: "33333333-3333-4333-8333-333333333333", profileVersion: 1,
    configurationComplete: false, components: [used] }];
  return { installed, used, profiles };
}
function whisperWithVad(): OfficialModelOption {
  const option = structuredClone(options[0]);
  option.review.manifest.engine = "whisper.cpp";
  for (const file of option.review.manifest.files) if (file.role === "llm") file.role = "asr";
  option.review.manifest.files.push({ relativePath: "models/vad.bin", role: "vad", expectedSize: 123, expectedHash: "e".repeat(64) });
  return option;
}

test("rebuilding a shared runtime archive does not turn applied checked models into downloads or VAD upgrades", () => {
  for (const option of [whisperWithVad(), options.find(o => o.review.manifest.engine === "sherpa-onnx")!, options[0]]) {
    const { installed, used, profiles } = checkedSelection(option);
    const choice = modelChoices(installed.engine, [installed], [option], profiles)[0];
    assert.equal(modelChoiceAction(choice, [installed], used), "in_use");
    // Keep runtime admission strict even when the applied selection is usable.
    if (installed.engine !== "llama.cpp") assert.equal(choice.entry, undefined);
  }
});

test("an installed VAD with changed transport metadata offers runtime preparation, never a nonexistent missing VAD", () => {
  const option = whisperWithVad(), { installed } = checkedSelection(option);
  const choice = modelChoices(installed.engine, [installed], [option], [])[0];
  assert.equal(modelChoiceAction(choice, [installed]), "update_runtime");
  installed.modelFiles = installed.modelFiles!.filter(file => file.role !== "vad");
  assert.equal(modelChoiceAction(choice, [installed]), "add_vad");
  installed.modelFiles.push({ role: "vad", size: 123, sha256: "f".repeat(64) });
  assert.equal(modelChoiceAction(choice, [installed]), "add_vad", "different VAD bytes cannot satisfy the required model");
});

test("applied state requires matching settings, current exact compatibility evidence and available complete model files", () => {
  const option = whisperWithVad(), { installed, used, profiles } = checkedSelection(option);
  const choice = modelChoices(installed.engine, [installed], [option], profiles)[0];
  for (const patch of [{ settingsMatch: false }, { compatibilityCurrent: false }, { checkOperationId: "44444444-4444-4444-8444-444444444444" }]) {
    assert.notEqual(modelChoiceAction(choice, [installed], { ...used, ...patch }), "in_use");
  }
  assert.notEqual(modelChoiceAction(choice, [{ ...installed, compatibility: null }], used), "in_use");
  assert.equal(modelChoiceAction(choice, [{ ...installed, bodyRemoved: true }], used), "download");
  assert.equal(modelChoiceAction(choice, [{ ...installed, placementRecorded: false }], used), "download");
  const withoutVad = { ...installed, modelFiles: installed.modelFiles!.filter(file => file.role !== "vad") };
  assert.equal(modelChoiceAction(choice, [withoutVad], used), "add_vad");
});

test("another selected model and an unapplied current package retain their own actions", () => {
  const option = whisperWithVad(), { installed, used, profiles } = checkedSelection(option);
  const current = { ...installed, download: { ...installed.download!, recipeHash: option.review.recipeHash } };
  const choice = modelChoices(current.engine, [current], [option], profiles)[0];
  assert.equal(modelChoiceAction(choice, [current], { ...used, packageId: "55555555-5555-4555-8555-555555555555" }), "use");
  const other = structuredClone(option); other.id = "other-model"; other.review.recipeHash = "f".repeat(64);
  other.review.manifest.files.find(file => file.role === "asr")!.expectedHash = "c".repeat(64);
  const different = modelChoices(installed.engine, [installed], [other], profiles).find(c => c.value === "official:other-model");
  assert.equal(modelChoiceAction(different, [installed], used), "download");
});
