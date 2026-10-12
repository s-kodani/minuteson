import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const panel=readFileSync(new URL("../src/workspace-panel.tsx",import.meta.url),"utf8");

test("panel trigger records and focuses the actual entry, independent of native mouse focus",()=>{
  assert.match(panel,/onClick=\{event => \{ event.currentTarget.focus\(\{ preventScroll:true \}\); reveal\(event.currentTarget\); \}\}/);
  assert.match(panel,/restoreTarget=\{\(\) => restore.current\?\.\(\) \?\? opener.current \?\? trigger.current\}/);
});

test("wide panel entry moves into the close control only for its own lost trigger focus",()=>{
  assert.match(panel,/const opened = !previousOpen.current && open;/);
  assert.match(panel,/ref=\{closeEntry\}/);
  assert.match(panel,/if \(opened\) \{/);
  assert.match(panel,/opener.current === trigger.current && \(active === opener.current \|\| active === document.body\)/);
  assert.match(panel,/closeEntry.current\?\.focus\(\{ preventScroll:true \}\)/);
  assert.match(panel,/document.querySelector\("dialog\[open\]"\)/);
  assert.match(panel,/cancelAnimationFrame\(frame\)/);
});
