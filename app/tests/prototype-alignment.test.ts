import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

test("prototype styling loads after the functional workspace styles", () => {
  const main = source("../src/main.tsx");
  assert.ok(main.indexOf('import "./prototype-alignment.css"') > main.indexOf('import "./style.css"'));
  assert.ok(main.includes('("meetings")'));
  assert.doesNotMatch(main, /DESKTOP FOUNDATION|className="next-grid"/);
  assert.match(main, /<table className="meeting-table">/);
  assert.match(main, /aria-label="会議名を検索"/);
  assert.doesNotMatch(main, /<ThemeSelection|aria-label="概要"/);
  assert.match(main, /screen === "settings" && <ThemeSettingsPanel/);
});
test("floating surfaces round every corner and do not use a thick square toast stripe", () => {
  const css = source("../src/prototype-alignment.css");
  assert.match(css, /--floating-radius:16px/);
  assert.match(css, /\.workspace-drawer,\.workspace-drawer-right \{ border-radius:var\(--floating-radius\)/);
  assert.match(css, /\.toast-card \{[^}]*border:1px solid var\(--border\)[^}]*overflow:hidden/);
  assert.match(css, /\.toast-host \{[^}]*padding:0/);
  assert.match(css, /\.toast-host \.mon-notification-stack \{ z-index:30/);
  assert.match(css, /\.recorder-info-window \{[^}]*overflow:auto[^}]*border-radius:12px/);
});
test("macOS clips only the three detached native windows using public selectors", () => {
  const native = source("../src-tauri/src/rounded_windows.rs");
  assert.match(native, /\["recorder", "recorder-info", "recording-confirmation"\]/);
  assert.match(native, /MainThreadMarker::new\(\)/);
  assert.match(native, /setCornerRadius: 12\.0_f64/);
  assert.match(native, /setMasksToBounds: true/);
  assert.match(native, /setOpaque: false/);
  assert.doesNotMatch(native, /setValue_forKey|setValue:.*forKey/);
  const setup = source("../src-tauri/src/lib.rs");
  assert.match(setup, /#\[cfg\(target_os = "macos"\)\]\s*rounded_windows::configure\(app.handle\(\)\)\?/);
  for (const file of ["tauri.conf.json", "tauri.qa.conf.json"]) {
    assert.notEqual(JSON.parse(source(`../src-tauri/${file}`)).app.macOSPrivateApi, true);
  }
});
test("model selectors keep button metrics without WebKit native sizing", () => {
  const css = source("../src/prototype-alignment.css");
  assert.match(css, /\.model-selection-row select \{[^}]*height:40px[^}]*box-sizing:border-box[^}]*padding:9px 32px 9px 12px/);
  assert.match(css, /\.capture-input-selection select,(?:\.active-microphone-selection select,)?\.model-selection-row select,\.voice-speaker select \{\s*-webkit-appearance:none; appearance:none/);
  assert.match(css, /@media\(forced-colors:active\) \{\s*[^}]*\.model-selection-row select,\.voice-speaker select \{ -webkit-appearance:auto; appearance:auto; background-image:none/);
  assert.match(css, /\.model-selection-row>button,\.model-selection-row select \{ min-height:40px; font-family:inherit; font-size:13px; line-height:20px/);
});
