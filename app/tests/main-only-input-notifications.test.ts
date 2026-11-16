import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
test('both recording windows read current status without receiving or acknowledging input notifications',()=>{
  const source=readFileSync(new URL('../src/recording-input-status.tsx',import.meta.url),'utf8');
  assert.match(source,/invoke\("get_recording_input_status"/);
  assert.doesNotMatch(source,/RecordingInputQueue|ToastNotice|receive_recording_input_notice|render_recording_input_notice|close_recording_input_notice/);
  assert.doesNotMatch(source,/mini-input-notice|mini-input-receipt-pending|noticeElementVisible/);
});
