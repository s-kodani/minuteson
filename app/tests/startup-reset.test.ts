import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { renderToStaticMarkup } from "react-dom/server";
import { checkedResetPreview, checkedResetSnapshot, resetRequest, type ResetPreview } from "../src/startup-reset-state.ts";
import { acceptStartupReport } from "../src/startup-attention-state.ts";
const id = "12345678-1234-4123-8123-123456789012";
const preview = (windows = false): ResetPreview => ({ availability: { available: true, reason: null }, operationId: id, meetingRoot: windows ? "C:\\Users\\私\\MON" : "/private/会議", archivePath: windows ? `D:\\退避\\MON-reset-20261027-${id}` : `/private/退避/MON-reset-20261027-${id}` });
test("reset requests carry only native operation IDs and the archive choice", () => {
  for (const windows of [false, true]) {
    const p = preview(windows);
    assert.doesNotThrow(() => checkedResetPreview(p));
    assert.deepEqual(resetRequest(p, true), { operationId: id, archive: true });
    assert.deepEqual(resetRequest({ ...p, archivePath: null }, false), { operationId: id, archive: false });
    assert.throws(() => resetRequest({ ...p, archivePath: null }, true));
    for (const meetingRoot of ["relative", "", "/bad\0path"]) assert.throws(() => checkedResetPreview({ ...p, meetingRoot }));
    assert.throws(() => checkedResetPreview({ ...p, availability: { available: false, reason: "使用中" } }));
  }
});
test("reset progress cannot unlock startup or overwrite its diagnostic", () => {
  const initial = { schemaVersion: 1, platform: "macos", phase: "blocked", failure: { stage: "storage_admission", message: "旧形式DB", messageTruncated: false, meetingRoot: "/private/会議", configRoot: "/private/config" } };
  const report = acceptStartupReport(null, initial);
  const changed = acceptStartupReport(report, { ...report, reset: { available: true, reason: null } });
  assert.deepEqual(changed.failure, report.failure);
  assert.throws(() => acceptStartupReport(changed, { ...changed, failure: { ...changed.failure, message: "changed" } }));
  assert.throws(() => acceptStartupReport(changed, { ...changed, phase: "ready", failure: null }));
  assert.throws(() => checkedResetSnapshot({ operationId: id, busy: true, completed: true, message: "", error: null }));
  assert.throws(() => checkedResetSnapshot({ operationId: null, busy: true, completed: false, message: "", error: null }));
  assert.doesNotThrow(() => checkedResetSnapshot({ operationId: id, busy: true, completed: false, message: "退避しています", error: null }));
});
const bundle = await build({ stdin: { contents: readFileSync(new URL("../src/startup-reset-form.tsx", import.meta.url), "utf8"), loader: "tsx", resolveDir: fileURLToPath(new URL("../src/", import.meta.url)) }, bundle: true, write: false, platform: "node", format: "esm", jsx: "automatic" });
const { StartupResetForm } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);
const props = { preview: preview(), archive: true, busy: false, selecting: false, message: "", error: "", onArchive() {}, onChoose() {}, onCancel() {}, onStart() {} };
test("actual reset form shows native paths, model deletion, two archive radios and the two final actions", () => {
  for (const windows of [false, true]) {
    const p = preview(windows);
    const html = renderToStaticMarkup(StartupResetForm({ ...props, preview: p }));
    assert.ok(html.includes("MinutesOnを起動し直します"));
    assert.match(html, /モデル本体とダウンロード途中のモデルデータは、退避の有無にかかわらず削除/u);
    assert.ok(html.includes(p.meetingRoot!));assert.ok(html.includes(p.archivePath!));
    assert.equal((html.match(/type="radio"/gu) ?? []).length, 2);
    assert.equal((html.match(/checked=""/gu) ?? []).length, 1);
    assert.doesNotMatch(html, /type="checkbox"|同意|確認して/u);
    assert.match(html, />キャンセル<\/button>[\s\S]*>初期化して再起動<\/button>/u);
    const noArchive = renderToStaticMarkup(StartupResetForm({ ...props, preview: p, archive: false }));
    assert.equal((noArchive.match(/<button\b/gu) ?? []).length, 2);
    assert.match(noArchive, /モデル本体とダウンロード途中/u);
    const busy = renderToStaticMarkup(StartupResetForm({ ...props, busy: true }));
    assert.equal((busy.match(/<button[^>]*disabled/gu) ?? []).length, 3);
  }
  const qa = renderToStaticMarkup(StartupResetForm({ ...props, productName: "MinutesOn QA", accepted: true }));
  assert.ok(qa.includes("MinutesOn QAを起動し直します"));
  assert.ok(qa.includes("MinutesOn QAを再起動"));
});
test("reset IPC is startup-only and available only to the main window", () => {
  const native = readFileSync(new URL("../src-tauri/src/startup_attention.rs", import.meta.url), "utf8");
  for (const command of ["get_startup_reset_preview", "choose_startup_reset_backup_directory", "start_startup_reset", "get_startup_reset_state"]) assert.ok(native.includes(`"${command}"`));
  const ipc = readFileSync(new URL("../src-tauri/src/startup_reset.rs", import.meta.url), "utf8");
  assert.match(ipc, /admit_command\(window.label\(\), "get_startup_reset_state"\)/u);
  assert.doesNotMatch(ipc, /message\.contains|削除対象DBが現行形式/u);
});
