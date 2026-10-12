import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const exportsSource = readFileSync(new URL("../src-tauri/src/exports.rs", import.meta.url), "utf8");
const dialog = exportsSource.slice(exportsSource.indexOf("pub(crate) async fn save_dialog_with("), exportsSource.indexOf("#[cfg(test)]"));

test("native save panel configures file types before assigning the full default filename", () => {
  const types = dialog.indexOf("panel.setAllowedFileTypes(");
  const filename = dialog.indexOf("panel.setNameFieldStringValue(");
  const present = dialog.indexOf("panel.runModal()");
  assert.ok(types >= 0 && filename > types && present > filename);
  assert.match(dialog, /setNameFieldStringValue\(&NSString::from_str\(&filename\)\)/);
  assert.match(dialog, /setAllowsOtherFileTypes\(false\)/);
});

test("shared save panel preserves the selected URL and safe cancellation for both output types", () => {
  assert.match(exportsSource, /save_dialog_with\(window, filename, "議事録をMarkdownで保存", "md"\)/);
  const backup = readFileSync(new URL("../src-tauri/src/data_controller/dialog.rs", import.meta.url), "utf8");
  assert.match(backup, /"MON-\{\}\.monbackup"/);
  assert.match(backup, /crate::exports::save_dialog_with\(/);
  assert.match(dialog, /response == NSModalResponseCancel[\s\S]*?return Ok\(None\)/);
  assert.match(dialog, /url\.isFileURL\(\)/);
  assert.match(dialog, /Ok\(Some\(PathBuf::from\(path\.to_string\(\)\)\)\)/);
  assert.doesNotMatch(dialog, /trim_end_matches|set_extension|replace\(/);
});

test("Markdown panel returns focus only after a known native response and enabled render", () => {
  const panel=readFileSync(new URL("../src/minutes.tsx",import.meta.url),"utf8");
  assert.doesNotMatch(panel,/"export_minutes_markdown"/);
  assert.match(panel,/"export_plain_minutes_markdown"/);
  assert.match(panel,/"get_plain_minutes_export_status"/);
  assert.match(panel,/!returnExportFocus.current \|\| locked \|\| busy \|\| exportState.busy \|\| !visible/);
  assert.match(panel,/target\.focus\(\{ preventScroll: true \}\)/);
  assert.match(panel,/!target\.closest\("\[hidden\],\[inert\]"\)/);
});

test("plain Markdown native output keeps detached admission and result checks cannot reopen a save panel", () => {
  const source=readFileSync(new URL("../src-tauri/src/lib.rs",import.meta.url),"utf8");
  const output=source.slice(source.indexOf("async fn export_plain_minutes_markdown("),source.indexOf("async fn get_plain_minutes_export_status("));
  assert.match(output,/minutes_app\(&window\)/);
  assert.match(output,/require_data_writable\(\)/);
  assert.match(output,/begin_export\(\)/);
  assert.match(output,/tauri::async_runtime::spawn\(async move[\s\S]*let _admission = admission;[\s\S]*let _lease = lease;/);
  assert.ok(output.indexOf("existing_markdown_export")<output.indexOf("exports::save_dialog"));
  const status=source.slice(source.indexOf("async fn get_plain_minutes_export_status("),source.indexOf("async fn get_minutes_export_status("));
  assert.ok(status.indexOf("let was_active")<status.indexOf("existing_markdown_export"));
  assert.match(status,/active: was_active \|\| app\.state::<exports::ExportController>\(\)\.active\(\)/);
  assert.doesNotMatch(status,/save_dialog|write_markdown_export|prepare_markdown_export/);
});

test("backup selection focus waits for its acknowledged token, terminal state and enabled visible UI", () => {
  const panel=readFileSync(new URL("../src/data-save.tsx",import.meta.url),"utf8");
  assert.match(panel,/choose\("backup", false, event.currentTarget\)/);
  assert.match(panel,/choose\("restore", false, event.currentTarget\)/);
  assert.match(panel,/button\?\.focus\(\{ preventScroll: true \}\)/);
  assert.match(panel,/if \(selected.token && selectionOpener\) selectionFocusReturn.current = \{ token: selected.token, target: selectionOpener \}/);
  assert.match(panel,/if \(!entry \|\| open \|\| !canAct \|\| !visible \|\| !s\) return/);
  assert.match(panel,/if \(s.token !== entry.token\) \{ selectionFocusReturn.current = null; return; \}/);
  assert.match(panel,/if \(!\["idle", "failed", "cancelled", "completed"\].includes\(s.phase\)\) return/);
  assert.match(panel,/target.closest\("\[hidden\],\[inert\]"\) \|\| modal && !modal.contains\(target\)/);
  assert.match(panel,/active === document.body \|\| active === target \|\| !active\?\.isConnected/);
  const focus=panel.slice(panel.indexOf("  useLayoutEffect(() => {"),panel.indexOf("  const requestReady"));
  assert.doesNotMatch(focus,/invoke\(|choose\(|send\(/);
});
