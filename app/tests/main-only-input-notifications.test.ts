import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
test('mini telemetry cannot receive or acknowledge notifications it does not present',()=>{
  const source=readFileSync(new URL('../src/recording-input-status.tsx',import.meta.url),'utf8');
  const start=source.indexOf('if(windowLabel==="recorder")');
  const end=source.indexOf('const attempts=queue.observe',start);
  assert.ok(start>=0&&end>start);
  const branch=source.slice(start,end);
  assert.match(branch,/notices:\[\].*return;/);
  assert.doesNotMatch(branch,/observe\(|deliver\(|receive\(|rendered\(/);
  assert.match(source,/windowLabel:"main"\}\)\{/);
  assert.doesNotMatch(source,/mini-input-notice|mini-input-receipt-pending|noticeElementVisible/);
});
