import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
test("recording operation failures keep notifications while acquisition changes use status only",()=>{
  const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");
  const operation=source("recorder-operation-notices.tsx").split("export function RecorderOperationNotifications")[1];
  assert.doesNotMatch(operation,/className="issue-status"|<InfoPopover|<section/);
  assert.match(operation,/operationNames\[e.source\].*e.message/);
  assert.match(operation,/dismiss_recorder_operation_notice/);
  const input=source("recording-input-status.tsx");
  assert.doesNotMatch(input,/ToastNotice|RecordingInputQueue|render_recording_input_notice/);
  assert.match(input,/data-input-state=\{value.state\}/);
});
