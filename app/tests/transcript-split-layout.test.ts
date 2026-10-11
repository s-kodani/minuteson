import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/transcript.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("../src/prototype-alignment.css", import.meta.url), "utf8");
const split = source.slice(source.indexOf("<CommonModal ref={splitDialog}"), source.indexOf("<CommonModal ref={addDialog}"));

test("split pairs related fields without changing save and retry guards", () => {
  assert.equal((split.match(/className="split-field-row"/g) ?? []).length, 2);
  assert.match(split, /disabled=\{busy\|\|pendingSplit\}/);
  assert.match(split, /disabled=\{!editable\|\|pendingSplit\|\|!!splitError\|\|splitAt===null\}/);
  assert.match(split, /splitUtterance\(analysis,p,selected.id,at,splitOffset,ids,assigned\)/);
  assert.match(split, /saveQueue.retry\(\)/);
  assert.match(split, /className="small-secondary" disabled=\{busy\} data-dialog-cancel/);
});

test("split modal limits width and supplies equal controls, preview spacing and mobile stacking", () => {
  assert.match(css, /\.common-modal.split-dialog \{ width:min\(760px,calc\(100vw - 32px\)\); max-width:760px/);
  assert.match(css, /\.split-field-row \{ display:grid; grid-template-columns:repeat\(2,minmax\(0,1fr\)\); gap:16px/);
  assert.match(css, /\.split-field-row :is\(input,select\).*min-height:40px.*font-size:13px/);
  assert.match(css, /\.split-preview section \{ display:grid; align-content:start; gap:8px/);
  assert.match(css, /@media\(max-width:600px\) \{ \.split-field-row \{ grid-template-columns:minmax\(0,1fr\)/);
});

test("split restores cancellation to its entry and saved split to the replacement row", () => {
  assert.match(source, /splitOpener.current=event.currentTarget;splitFirstPart.current=null/);
  assert.match(split, /restoreReady=\{!busy\} restoreTarget=\{/);
  assert.match(split, /splitOpener.current\?\.getClientRects\(\).length\?splitOpener.current/);
  assert.match(split, /CSS.escape\(splitFirstPart.current\)/);
  assert.match(split, /splitFirstPart.current=ids\[0\]/);
});
