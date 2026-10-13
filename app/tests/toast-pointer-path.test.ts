import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
test('toast stack hover follows pointer events rather than sticky WebKit hover state',()=>{
  const source=readFileSync(new URL('../src/toast.tsx',import.meta.url),'utf8');
  assert.match(source,/const hover = \(event: PointerEvent\)/);
  assert.match(source,/event.composedPath\(\).includes\(host.current\)/);
  assert.doesNotMatch(source,/host.current\?\.matches\(":hover"\)/);
  assert.match(source,/reading.current.hover !== hovered/);
});
