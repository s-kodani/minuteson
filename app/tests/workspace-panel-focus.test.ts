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

test("reader and rich evidence forward the actual clicked chip before opening its reference panel",()=>{
  const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");
  const reader=source("minutes-document.tsx"),field=source("minutes-rich-field.tsx"),editor=source("minutes-editor.tsx");
  assert.match(reader,/onClick=\{event => showEvidence\(e.id, event.currentTarget\)\}/);
  assert.match(reader,/const showEvidence = \(id: string, from\?: HTMLElement\)/);
  assert.match(reader,/evidenceOpener.current = from \?\? \(document.activeElement/);
  assert.match(reader,/from.focus\(\{ preventScroll:true \}\)/);
  assert.match(field,/onEvidence\?\.\(chip.dataset.evidenceId!, chip\)/);
  assert.match(field,/onEvidence\(chip.dataset.evidenceId!, chip\)/);
  assert.match(editor,/onEvidence=\{\(id,opener\) => documentHandle.current\?\.showEvidence\(id,opener\)\}/);
});
