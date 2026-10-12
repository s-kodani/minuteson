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
  assert.match(hide,/\.hide\(\)/);
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
test("mini close stays usable while stop confirmation keeps stop controls guarded",()=>{
  const sourceWindow=source("../src-tauri/src/recording_confirmation_window.rs");
  const present=sourceWindow.split("let present =")[1].split("if let Err(error) = present()")[0];
  assert.match(present,/for label in \["main"\]/);
  assert.doesNotMatch(present,/\["main", "recorder"\]/);
  assert.match(sourceWindow,/window.label\(\) != "recorder" \|\| window.is_visible\(\).unwrap_or\(false\)/);
  const permission=JSON.parse(source("../src-tauri/capabilities/recorder.json"));
  assert.deepEqual(permission.windows,["recorder"]);
  assert.ok(permission.permissions.includes("core:window:allow-start-dragging"));
});
test("mini issue badge uses a separate grid row and adjacent buttons stay top aligned",()=>{
  const base=source("../src/style.css"), mini=source("../src/recorder-panel.css");
  assert.match(base,/\.recorder-info-trigger \{ display:inline-grid; position:relative; justify-items:center; gap:4px; \}/);
  const badge=base.match(/\.recorder-operation-badge \{([^}]*)\}/)?.[1]??"";
  assert.ok(badge.includes("line-height:10px"));
  assert.doesNotMatch(badge,/position:absolute|bottom:|left:|width:/);
  assert.match(mini,/\.mini-shell \.mini-bottom \{ flex:none; gap:4px; margin:0; align-items:flex-start; \}/);
});
