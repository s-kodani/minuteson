import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

test("settings no longer expose interrupted/deleted data management or manual recovery", () => {
  const main = read("../src/main.tsx");
  assert.doesNotMatch(main, /中断・削除の管理|settingsManagementOpen|\bRecoveryPanel\b|PartialRecordingPanel|ModelBodyDeletionPanel|useModelBodyDeletion/);
  for (const file of ["recovery-panel", "partial-recording-panel", "partial-recording-results", "partial-recording-summary", "model-body-deletion"]) {
    assert.equal(existsSync(new URL(`../src/${file}.tsx`, import.meta.url)), false);
  }
  assert.doesNotMatch(read("../src/data-operation.ts"), /get_partial_recording_items|get_partial_recording_results/);
});

test("normal meeting/audio deletion keeps consent modals without administrative panels", () => {
  for (const file of ["meeting-deletion", "audio-deletion"]) {
    const source = read(`../src/${file}.tsx`);
    assert.match(source, /<CommonModal portal/);
    assert.match(source, /type="checkbox" checked=\{c.understood\}/);
    assert.match(source, /<IssueNotice/);
    assert.doesNotMatch(source, /<section|未完了.*削除の対象を確認|quietMaintenance|visible: boolean/);
  }
});

test("manual recovery/model deletion IPC is removed and startup maintenance is off the UI thread", () => {
  const native = read("../src-tauri/src/lib.rs");
  for (const name of ["get_recovery_items", "review_recovery_item", "get_partial_recording_items", "review_partial_recording", "get_partial_recording_results", "review_model_body_deletion", "review_pending_model_deletion", "get_model_deletion_paths"]) assert.doesNotMatch(native, new RegExp(`\\b${name}\\b`));
  assert.match(native, /std::thread::spawn\(move \|\| \{\s*#\[cfg\(unix\)\]\s*if let Err\(error\) = data_controller::finish_interrupted_deletions/);
  const automatic = read("../src-tauri/src/data_controller/automatic.rs");
  assert.match(automatic, /guard.selection.is_some\(\)/);
  assert.match(automatic, /Admission::acquire_deletion/);
  assert.match(automatic, /Admission::acquire_audio_deletion/);
  assert.match(automatic, /Admission::acquire_model_deletion/);
  assert.match(automatic, /catch_unwind/);
});
