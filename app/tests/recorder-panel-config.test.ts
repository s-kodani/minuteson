import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
const source=(path:string)=>readFileSync(new URL(path,import.meta.url),"utf8");
test("mini hide is authorized only for recorder and independent of recording locks or stopping",()=>{
  const lib=source("../src-tauri/src/lib.rs"), window=source("../src-tauri/src/recording_confirmation_window.rs");
  const command=lib.split("fn hide_recorder_panel(")[1].split("#[tauri::command]")[0];
  assert.match(command,/window.label\(\) != "recorder"/);
  assert.match(command,/hide_recorder\(window.app_handle\(\)\)/);
  const hide=window.split("pub fn hide_recorder(")[1];
  assert.doesNotMatch(hide,/RecorderController|SessionStore|session_changed|with_transition|request_recording_confirmation|decide_recording_confirmation/);
  assert.match(hide,/recorder_panel::hide\(&panel\)/);
  assert.match(hide,/main\.show\(\)/);
  assert.doesNotMatch(lib,/recorder_close_requested|hide_recorder_preview/);
});
test("mini always has an independent close action without disabling on active or unreadable session",()=>{
  const panel=source("../src/main.tsx").split("function RecorderPanel()")[1].split("function MainWindow()")[0];
  assert.match(panel,/label="ミニパネルを閉じる"/);
  assert.match(panel,/closeAction.run\("hide_recorder_panel"\)/);
  assert.match(panel,/disabled=\{closeAction.busy \|\| !isTauri\(\)\}/);
  assert.doesNotMatch(panel,/idle && <IconButton/);
});
test("mini close stays usable while non-modal stop popup keeps stop controls guarded",()=>{
  const sourceWindow=source("../src-tauri/src/recording_confirmation_window.rs");
  const present=sourceWindow.split("let present =")[1].split("if let Err(error) = present()")[0];
  assert.doesNotMatch(present,/set_enabled\(false\)/);
  assert.match(present,/popup_position\(/);
  assert.match(sourceWindow,/window.label\(\) != "recorder" \|\| crate::recorder_panel::may_present\(app\)/);
  assert.match(sourceWindow,/popup.set_focus\(\).*crate::recorder_panel::synchronize\(app\)/s);
  const permission=JSON.parse(source("../src-tauri/capabilities/recorder.json"));
  assert.deepEqual(permission.windows,["recorder"]);
  assert.ok(permission.permissions.includes("core:window:allow-start-dragging"));
});
test("native main focus and recording phase own mini visibility without a manual show command",()=>{
  const lib=source("../src-tauri/src/lib.rs"),recorder=source("../src-tauri/src/recorder.rs"),panel=source("../src-tauri/src/recorder_panel.rs");
  assert.match(lib,/WindowEvent::Focused\(focused\).*recorder_panel::main_focus_changed/s);
  const residency=source("../src-tauri/src/residency.rs");
  assert.match(residency,/window\.hide\(\).*recorder_panel::synchronize\(window.app_handle\(\)\)/s);
  assert.match(lib,/window.emit\("main-close-requested", serde_json::json!\(\{"quit": false\}\)\)/);
  assert.doesNotMatch(lib,/show_recorder_preview/);
  assert.match(recorder,/let _ = crate::recorder_panel::synchronize\(&handle\);\s*crate::recording_confirmation_window::session_changed\(&handle, &snapshot\)/);
  assert.doesNotMatch(recorder,/panel\.show\(\)/);
  assert.match(panel,/matches!\(snapshot.phase, RecordingPhase::Recording\)\s*&& !main_focused/);
  assert.match(panel,/RecordingPhase::Stopping,/);
  assert.match(panel,/if !state.wants_panel\(&snapshot, focused, confirming_elsewhere\) \{/);
  assert.match(panel,/decision.origin.as_deref\(\)/);
  assert.match(panel,/&& !confirming/);
  assert.match(panel,/\.dismissed\s*\.store\(true, Ordering::SeqCst\)/);
});
test("mini keeps icon controls without info, phase labels or inline diagnostics",()=>{
  const panel=source("../src/main.tsx").split("function RecorderPanel()")[1].split("function MainWindow()")[0];
  assert.doesNotMatch(panel,/RecorderInfoTrigger|mini-save-diagnostics|<strong>/);
  assert.match(panel,/aria-label=\{phaseLabel\(state\)\}/);
});
test("mini centers its six-dot grip without a separator and keeps controls in the header",()=>{
  const mini=source("../src/recorder-panel.css");
  assert.match(mini,/\.mini-shell \{[^}]*display:flex;[^}]*justify-content:center;/);
  assert.match(mini,/\.mini-shell \.mini-row \{[^}]*align-items:center;/);
  assert.match(mini,/\.mini-shell \.mini-drag \{[^}]*align-self:stretch;[^}]*width:14.4px;[^}]*border:0;/);
  assert.match(mini,/\.mini-shell \.mini-close\.icon-button \{ position:static; \}/);
  const panel=source("../src/main.tsx").split("function RecorderPanel()")[1].split("function MainWindow()")[0];
  assert.match(panel,/<header className="mini-header">[\s\S]*label="ミニパネルを閉じる"[\s\S]*<\/header>/);
  for(const file of ["tauri.conf.json","tauri.qa.conf.json"]){
    const mini=JSON.parse(source(`../src-tauri/${file}`)).app.windows.find((w:{label:string})=>w.label==="recorder");
    assert.equal(mini.width,227);assert.equal(mini.height,101);
  }
});
