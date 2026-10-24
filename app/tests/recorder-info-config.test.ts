import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
test("recorder explanation starts hidden and unfocused in production and QA", () => {
  for (const file of ["tauri.conf.json", "tauri.qa.conf.json"]) {
    const windows = JSON.parse(source(`../src-tauri/${file}`)).app.windows;
    const info = windows.find((window: { label: string }) => window.label === "recorder-info");
    assert.equal(info.url, "index.html?window=recorder-info");
    assert.equal(info.visible, false); assert.equal(info.focus, false);
    assert.equal(info.alwaysOnTop, true); assert.equal(info.skipTaskbar, true);
    assert.equal(info.width, 360); assert.equal(info.height, 340);
  }
});
test("explanation has its own role and does not mount recording or editing providers", () => {
  const cap = JSON.parse(source("../src-tauri/capabilities/recorder-info.json"));
  assert.deepEqual(cap.windows, ["recorder-info"]);
  assert.deepEqual(cap.permissions, ["core:default", "core:event:allow-listen", "recorder-info-commands"]);
  const ui = source("../src/main.tsx");
  assert.ok(ui.includes("isRecorderInfo ? <RecorderInfoWindow /> : <RecordingInputProvider"));
  const popup = source("../src/recorder-info.tsx").split("export function RecorderInfoWindow()")[1];
  assert.doesNotMatch(popup, /invoke\("(?:stop_recording|start_recording|save_|delete_|decide_recording_confirmation)/);
});
test("native confirmation dismisses explanation without restoring recorder focus", () => {
  const confirmation = source("../src-tauri/src/recording_confirmation_window.rs");
  assert.ok(confirmation.includes("crate::recorder_info::hide(app, false)?;"));
  assert.ok(confirmation.indexOf("crate::recorder_info::hide(app, false)?;") < confirmation.indexOf("popup.show()"));
  const native = source("../src-tauri/src/recorder_info.rs");
  assert.doesNotMatch(native, /RecorderController|state::<Storage>|SessionStore/);
});
test("native explanation transitions focus the WebView after its window", () => {
  const native = source("../src-tauri/src/recorder_info.rs");
  const focus = native.split("fn focus_content(")[1]?.split("pub fn hide(")[0] ?? "";
  assert.ok(focus.includes("window.set_focus()"));
  assert.match(focus, /#\[cfg\(any\(target_os = "macos",\s*windows\)\)\]/);
  assert.ok(focus.includes("let webview: &tauri::Webview = window.as_ref();"));
  assert.ok(focus.includes("webview.set_focus()"));
  assert.ok(focus.indexOf("window.set_focus()") < focus.indexOf("webview.set_focus()"));
  assert.ok(native.includes("focus_content(recorder.as_ref().unwrap())?;"));
  assert.ok(native.includes("if let Err(error) = focus_content(&popup)"));
  assert.match(native, /if restore \{\s*focus_content\(recorder\.as_ref\(\)\.unwrap\(\)\)\?;/);
});
test("mini explanation button reserves readable widths for both input labels", () => {
  const css = source("../src/style.css");
  assert.ok(css.includes(".mini-top:has(.mini-inputs) { gap:2px; }"));
  assert.ok(css.includes("grid-template-columns:minmax(75px,1.15fr) minmax(65px,1fr)"));
});

test("native explanation ignores focus loss while explicit close and lifecycle hiding remain", () => {
  const lib = source("../src-tauri/src/lib.rs");
  const events = lib.split('if window.label() == recorder_info::LABEL {')[1].split('if window.label() == "main"')[0];
  assert.doesNotMatch(events, /WindowEvent::Focused/);
  assert.match(events, /CloseRequested[\s\S]*recorder_info::hide\(window.app_handle\(\), true\)/);
  assert.match(source("../src-tauri/src/recorder_panel.rs"), /recorder_info::hide\(app, false\)/);
  assert.match(source("../src-tauri/src/recording_confirmation_window.rs"), /recorder_info::hide\(app, false\)/);
});
