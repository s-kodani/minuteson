import test from "node:test";
import assert from "node:assert/strict";
import { assertGuidedInspection, guidedModelSource, importRoles, type ImportEngine } from "../src/model-import-guide-state.ts";
import type { ModelInspectionReport } from "../src/model-inspection-state.ts";

test("guided imports create each engine's declaration with no publisher or readiness claim", () => {
  for (const engine of Object.keys(importRoles) as ImportEngine[]) {
    const files = importRoles[engine].map(item => ({ role: item.role, path: `/Users/test/models/${item.role}` }));
    const source = guidedModelSource(engine, files);
    assert.equal(source.directory, "/Users/test/models");
    assert.equal(source.manifest.engine, engine);
    assert.equal(source.manifest.engineVersion, "未申告");
    assert.equal(source.manifest.modelVersion, "未申告");
    assert.equal(source.manifest.files.length, files.length);
    assert.ok(source.manifest.files.every(file => file.expectedHash === null && file.expectedSize === null));
    assert.match(source.manifest.licenseRef, /別途確認/);
  }
});
test("files can stay in separate folders while runtime library relative placement is preserved", () => {
  const source = guidedModelSource("llama.cpp", [
    { role: "executable", path: "/Users/test/engine/bin/llama-cli" },
    { role: "runtime_library", path: "/Users/test/engine/lib/libllama.dylib" },
    { role: "llm", path: "/Users/test/weights/日本語.gguf" },
  ]);
  assert.equal(source.directory, "/Users/test");
  assert.deepEqual(source.manifest.files.map(file => file.relativePath), ["engine/bin/llama-cli", "engine/lib/libllama.dylib", "weights/日本語.gguf"]);
  assert.equal(source.manifest.modelName, "日本語.gguf");
});
test("guided imports reject missing, duplicate, wrong-engine and traversing selections", () => {
  const files = [{ role: "executable" as const, path: "/models/cli" }, { role: "llm" as const, path: "/models/model.gguf" }];
  assert.throws(() => guidedModelSource("llama.cpp", files.slice(0, 1)));
  assert.throws(() => guidedModelSource("whisper.cpp", files));
  assert.throws(() => guidedModelSource("llama.cpp", [...files, files[1]]));
  assert.throws(() => guidedModelSource("llama.cpp", [files[0], { ...files[1], path: files[0].path }]));
  assert.throws(() => guidedModelSource("llama.cpp", [files[0], { ...files[1], path: "/models/CLI" }]));
  for (const path of ["model.gguf", "/models/../model.gguf", "/models//model.gguf", "/models/./model.gguf", "/models/a\0", "/models/mon-model.json"]) assert.throws(() => guidedModelSource("llama.cpp", [files[0], { ...files[1], path }]));
});
test("guided source enumerates selected files only, including disjoint roots", () => {
  const source = guidedModelSource("whisper.cpp", [{ role: "executable", path: "/engine/cli" }, { role: "asr", path: "/models/tiny.bin" }]);
  assert.equal(source.directory, "/");
  assert.deepEqual(source.manifest.files.map(file => file.relativePath), ["engine/cli", "models/tiny.bin"]);
});

test("guided results bind every selected role and unverified declaration before offering copy", () => {
  const source = guidedModelSource("llama.cpp", [{ role: "executable", path: "/models/cli" }, { role: "llm", path: "/models/model.gguf" }]);
  const report: ModelInspectionReport = { manifest: structuredClone(source.manifest), manifestHash: "a".repeat(64), reviewHash: "b".repeat(64), totalBytes: 20, engineCompatibility: "not_verified", publisherAuthenticity: "not_verified", installed: false, ready: false, files: source.manifest.files.map(file => ({ relativePath: file.relativePath, role: file.role, actualSize: 10, actualHash: "c".repeat(64), expectedSizeChecked: false, expectedHashChecked: false })) };
  assert.doesNotThrow(() => assertGuidedInspection(report, source));
  for (const mutate of [
    (r: ModelInspectionReport) => { r.manifest.modelName = "other"; },
    (r: ModelInspectionReport) => { r.files[1].role = "asr"; },
    (r: ModelInspectionReport) => { r.files[0].relativePath = "other"; },
    (r: ModelInspectionReport) => { r.files[0].expectedHashChecked = true; },
    (r: ModelInspectionReport) => { r.totalBytes = 21; },
    (r: ModelInspectionReport) => { r.files[0].actualSize = NaN; },
    (r: ModelInspectionReport) => { r.files.pop(); },
    (r: ModelInspectionReport) => { r.reviewHash = "r"; },
    (r: ModelInspectionReport) => { (r.manifest as typeof source.manifest).files[0].relativePath = "other"; },
  ]) { const changed = structuredClone(report); mutate(changed); assert.throws(() => assertGuidedInspection(changed, source)); }
});
