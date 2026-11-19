import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
const lib=readFileSync(new URL("../src-tauri/src/lib.rs",import.meta.url),"utf8");
test("capture polling and stopping dispatch lock waits and file reads off the native UI thread",()=>{
  for(const name of ["get_recording_levels","get_recording_writes","get_recording_input_status","get_capture_test_levels","get_capture_test_snapshot","cancel_capture_test","receive_recording_input_notice","render_recording_input_notice","close_recording_input_notice"]){
    const body=lib.split(`async fn ${name}(`)[1]?.split("#[tauri::command]")[0];
    assert.ok(body,`${name} must be async`);
    assert.match(body,/spawn_blocking/);
    assert.match(body,/window\.label\(\)/);
  }
  const confirmation=readFileSync(new URL("../src-tauri/src/recording_confirmation_window.rs",import.meta.url),"utf8");
  assert.match(confirmation,/pub async fn decide_recording_confirmation/);
  assert.match(confirmation,/spawn_blocking\(move \|\| \{\s*decide_recording_confirmation_blocking/);
});
test("recording recovery retries acquire the native window transition off the UI thread",()=>{
  const recovery=readFileSync(new URL("../src-tauri/src/recording_recovery.rs",import.meta.url),"utf8");
  const command=recovery.split("pub(crate) async fn retry_recording_recovery(")[1]?.split("fn retry_recording_recovery_blocking(")[0];
  assert.ok(command,"recovery retry must dispatch asynchronously");
  assert.match(command,/spawn_blocking\(move \|\| \{\s*retry_recording_recovery_blocking\(window, session_id\)/);
  assert.doesNotMatch(command,/with_transition/);
  const blocking=recovery.split("fn retry_recording_recovery_blocking(")[1]?.split("#[tauri::command]")[0];
  assert.ok(blocking);
  assert.match(blocking,/RecorderController::with_transition/);
  assert.match(blocking,/window\.label\(\) != "main"/);
  const registry=JSON.parse(readFileSync(new URL("../src-tauri/command_registry.json",import.meta.url),"utf8"));
  assert.equal(registry.commands.find((command:{name:string})=>command.name==="retry_recording_recovery").signature.async,true);
});
test("input preparation has no permanent permission-help button or modal",()=>{
  const source=readFileSync(new URL("../src/capture-inputs.tsx",import.meta.url),"utf8");
  assert.doesNotMatch(source,/録音の権限を確認|OptionsModal|拒否されたとは判断できません/);
  assert.match(source,/const permissionRequired=permission==="denied"\|\|permission==="restricted"/);
  assert.match(source,/<ErrorNotice[^>]*type="permission"[^>]*active=\{permissionRequired\}/);
  assert.doesNotMatch(source,/<p role="alert">/);
  assert.match(source,/マイクの利用を許可してください/);
  assert.match(source,/macOSのシステム設定/);
  assert.doesNotMatch(source,/requestAccess|start_capture_test|start_recording/);
});
test("production recording and five-second tests select audio-only taps with no screen fallback",()=>{
  for(const file of ["recorder.rs","capture_test.rs"]){
    const native=readFileSync(new URL(`../src-tauri/src/${file}`,import.meta.url),"utf8");
    assert.match(native,/\.arg\("--backend"\)\s*\.arg\((?:crate::recorder::)?capture_backend\(\)\)/);
    assert.doesNotMatch(native,/\.arg\("screencapturekit"\)/);
  }
  const catalog=readFileSync(new URL("../../probes/macos-capture/Sources/MONMacOSCapture/CaptureApplications.swift",import.meta.url),"utf8");
  assert.doesNotMatch(catalog,/CGPreflightScreenCaptureAccess|CGRequestScreenCaptureAccess/);
  assert.match(catalog,/systemPermission: "unknown"/);
});
