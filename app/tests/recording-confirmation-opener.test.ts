import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),'utf8');
test('recording confirmations preserve the clicked opener on macOS without changing stop ownership',()=>{
  assert.match(source('recording-confirmation.tsx'),/request=async\(quit=false,source\?:HTMLElement\)/);
  assert.match(source('recording-confirmation.tsx'),/opener.current=source\?\?/);
  assert.match(source('recorder-panel.tsx'),/decision.request\(false,event.currentTarget\)/);
  assert.match(source('main.tsx'),/stopRecording\(event.currentTarget\)/);
  assert.match(source('recording-workspace.tsx'),/onStop\(event.currentTarget\)/);
});
