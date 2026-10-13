import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
test('non-modal help makes links reachable without changing OS keyboard settings or trapping focus',()=>{
  const source=readFileSync(new URL('../src/info-popover.tsx',import.meta.url),'utf8');
  assert.match(source,/event.key === "Tab".*!event.isComposing.*event.keyCode !== 229/);
  assert.match(source,/element.contains\(document.activeElement\)/);
  assert.match(source,/a\[href\]/);
  assert.match(source,/!node.closest\("\[hidden\],\[inert\]"\)/);
  assert.match(source,/next >= 0 && next < targets.length/);
  assert.doesNotMatch(source,/% targets.length|invoke\(/);
});
