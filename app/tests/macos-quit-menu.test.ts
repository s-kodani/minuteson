import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src-tauri/src/macos_menu.rs", import.meta.url), "utf8");
const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");

// Source contracts complement, not replace, a packaged macOS save/quit test.
test("macOS Quit uses a custom Cmd+Q action rather than direct native termination", () => {
  assert.match(source, /MenuItem::with_id\(app, GUARDED_QUIT, "終了", true, Some\("Cmd\+Q"\)\)/u);
  assert.doesNotMatch(source, /PredefinedMenuItem::quit|process::exit|terminate:/u);
  assert.match(source, /event\.id\(\)\.as_ref\(\) == GUARDED_QUIT/u);
  assert.match(source, /app\.exit\(0\)/u);
  assert.doesNotMatch(source, /ExitGuard.*store|finish_main_close\(/u);
});

test("custom Quit keeps standard macOS application and editing menu roles", () => {
  assert.match(source, /Menu::default\(app\.handle\(\)\)/u);
  for (const role of ["about", "services", "hide", "hide_others", "separator"]) {
    assert.ok(source.includes(`PredefinedMenuItem::${role}(`));
  }
  assert.match(source, /menu\.remove\(&original\)\?/u);
  assert.match(source, /menu\.insert\(&application, 0\)\?/u);
  assert.match(source, /app\.set_menu\(menu\)\?/u);
  assert.match(lib, /#\[cfg\(target_os = "macos"\)\]\s+macos_menu::install\(app\)\?/u);
});

test("Quit still enters the existing fail-closed save and worker shutdown barrier", () => {
  const exit = lib.slice(lib.indexOf("if let RunEvent::ExitRequested"));
  assert.match(exit, /if !app\.state::<ExitGuard>\(\)\.0\.load\(Ordering::SeqCst\)/u);
  assert.match(exit, /api\.prevent_exit\(\)/u);
  assert.match(exit, /StopDecisionStore/u);
  assert.match(exit, /main\.emit\("main-close-requested", serde_json::json!\(\{"quit": true\}\)\)/u);
  assert.match(lib, /ExitGuard>\(\)\.0\.store\(true, Ordering::SeqCst\);\s+window\.app_handle\(\)\.exit\(0\)/u);
});
