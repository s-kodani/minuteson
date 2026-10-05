import test from "node:test";
import assert from "node:assert/strict";
import { assertModelDeletionPreview, modelDeletionEntryLabel, type ModelDeletionPreview, type ModelDeletionEntry } from "../src/model-deletion-state.ts";
const packageId = "11111111-1111-4111-8111-111111111111", token = "22222222-2222-4222-8222-222222222222";
const selected = { packageId, reviewHash: "a".repeat(64), modelName: "合成モデル", modelVersion: "1" };
const root: ModelDeletionEntry = { relativePath: "", directory: true, registered: true, present: true, bytes: null, sha256: null, changed: false };
const file: ModelDeletionEntry = { relativePath: "cli", directory: false, registered: true, present: true, bytes: 10, sha256: "b".repeat(64), changed: false };
const preview: ModelDeletionPreview = { schemaVersion: 1, inspectionId: token, ...selected, censusHash: "c".repeat(64), modelsRoot: "/app/models", entries: [root, file], bytes: 10, missingFiles: 0, changedFiles: 0, unregisteredFiles: 0, profileReferences: 2, jobReferences: 7, activeJobs: 0, executeAllowed: false };
const validate = (p: unknown) => assertModelDeletionPreview(p, token, selected);

test("model deletion observation binds token package review name and version, never execution authority", () => {
  assert.doesNotThrow(() => validate(preview));
  for (const patch of [{ schemaVersion: 2 }, { inspectionId: packageId }, { packageId: token }, { reviewHash: "d".repeat(64) }, { censusHash: "bad" }, { modelName: "other" }, { modelVersion: "2" }, { executeAllowed: true }, { executeAllowed: null }]) assert.throws(() => validate({ ...preview, ...patch }));
  for (const value of [undefined, null, [], "preview", {}, { ...preview, entries: null }]) assert.throws(() => validate(value));
});
test("model deletion totals come from all entries and reject negative unsafe or inconsistent counts", () => {
  for (const key of ["bytes", "missingFiles", "changedFiles", "unregisteredFiles", "profileReferences", "jobReferences", "activeJobs"]) for (const n of [-1, 0.5, Number.NaN, Number.MAX_SAFE_INTEGER + 1, "1"]) assert.throws(() => validate({ ...preview, [key]: n }));
  for (const patch of [{ bytes: 11 }, { missingFiles: 1 }, { changedFiles: 1 }, { unregisteredFiles: 1 }]) assert.throws(() => validate({ ...preview, ...patch }));
  assert.throws(() => validate({ ...preview, entries: [root, { ...file, bytes: Number.MAX_SAFE_INTEGER }, { ...file, relativePath: "other", bytes: 1 }], bytes: Number.MAX_SAFE_INTEGER }));
});
test("model deletion inventory cannot substitute unsafe paths duplicates or detached children", () => {
  for (const path of ["/outside", "../outside", "a/../b", "a/./b", "a//b", "a/", "C:outside", "a\\b", "bad\0", "x".repeat(256), Array(33).fill("a").join("/"), "missing/file"]) assert.throws(() => validate({ ...preview, entries: [root, { ...file, relativePath: path }] }));
  for (const entries of [[], [file], [root, file, file], [file, root], [{ ...root, registered: false }, file], [{ ...root, directory: false }, file], [root, { ...file, relativePath: "cli/child" }]]) assert.throws(() => validate({ ...preview, entries }));
  for (const modelsRoot of ["relative/models", "/app/../models", "/app/./models", "/app/models/", "/app//models", "/app/other", "/app\\models", "/app\0/models"]) assert.throws(() => validate({ ...preview, modelsRoot }));
});
test("model deletion absent changed and unknown classifications remain separate and consistent", () => {
  const entries = [root, { ...file, changed: true }, { ...file, relativePath: "missing", present: false, bytes: null, sha256: null }, { ...file, relativePath: "unknown", registered: false }];
  assert.doesNotThrow(() => validate({ ...preview, entries, bytes: 20, missingFiles: 1, changedFiles: 1, unregisteredFiles: 1 }));
  for (const patch of [{ bytes: null }, { sha256: null }, { sha256: "bad" }, { registered: false, changed: true }, { present: false }, { directory: true }, { changed: "yes" }, { present: null }]) assert.throws(() => validate({ ...preview, entries: [root, { ...file, ...patch }] }));
  assert.throws(() => validate({ ...preview, entries: [root, { ...file, registered: false, present: false, bytes: null, sha256: null }], bytes: 0, missingFiles: 1, unregisteredFiles: 1 }));
  assert.equal(modelDeletionEntryLabel(entries[1]), "登録内容から変更あり");
  assert.equal(modelDeletionEntryLabel(entries[2]), "欠落（配置記録のみ）");
  assert.equal(modelDeletionEntryLabel(entries[3]), "未登録ファイル");
  assert.equal(modelDeletionEntryLabel(root), "登録フォルダー");
});
test("model deletion handles entirely absent and partially placed packages without readiness claims", () => {
  assert.doesNotThrow(() => validate({ ...preview, entries: [{ ...root, present: false }, { ...file, present: false, bytes: null, sha256: null }], bytes: 0, missingFiles: 1 }));
  assert.throws(() => validate({ ...preview, entries: [{ ...root, present: false }, file] }));
  assert.doesNotThrow(() => validate({ ...preview, entries: [root, { ...root, relativePath: "weights" }, { ...file, relativePath: "weights/model.bin" }] }));
});
test("model deletion Unicode ordering matches native UTF-8 rather than UTF-16 sort", () => {
  const entries = [root, file, { ...file, relativePath: "\ue000", registered: false }, { ...file, relativePath: "\u{10000}", registered: false }];
  assert.doesNotThrow(() => validate({ ...preview, entries, bytes: 30, unregisteredFiles: 2 }));
  assert.throws(() => validate({ ...preview, entries: [root, file, entries[3], entries[2]], bytes: 30, unregisteredFiles: 2 }));
});
test("model deletion bounds inventory including folders while allowing the complete 256-entry review", () => {
  const entries = [root, ...Array.from({ length: 255 }, (_, n) => ({ ...file, relativePath: `file-${String(n).padStart(3, "0")}` }))];
  assert.doesNotThrow(() => validate({ ...preview, entries, bytes: 2550 }));
  assert.throws(() => validate({ ...preview, entries: [...entries, { ...file, relativePath: "last" }], bytes: 2560 }));
});
