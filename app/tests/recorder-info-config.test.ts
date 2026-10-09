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
  assert.deepEqual(cap.permissions, ["core:default", "core:event:allow-listen"]);
  const ui = source("../src/main.tsx");
  assert.ok(ui.includes("isRecorderInfo ? <RecorderInfoWindow /> : isRecordingConfirmation"));
  const popup = source("../src/recorder-info.tsx").split("export function RecorderInfoWindow()")[1];
  assert.doesNotMatch(popup, /invoke\("(?:stop_recording|start_recording|save_|delete_|decide_recording_confirmation)/);
});
test("native confirmation dismisses explanation without restoring recorder focus", () => {
  const confirmation = source("../src-tauri/src/recording_confirmation_window.rs");
  assert.ok(confirmation.includes("crate::recorder_info::hide(app, false)?;"));
  assert.ok(confirmation.indexOf("crate::recorder_info::hide(app, false)?;") < confirmation.indexOf("modal.show()"));
  const native = source("../src-tauri/src/recorder_info.rs");
  assert.doesNotMatch(native, /RecorderController|state::<Storage>|SessionStore/);
});
