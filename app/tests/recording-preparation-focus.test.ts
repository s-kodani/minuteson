import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";

const workspace=readFileSync(new URL("../src/recording-workspace.tsx",import.meta.url),"utf8");
const main=readFileSync(new URL("../src/main.tsx",import.meta.url),"utf8");

test("preparation focuses its mounted title after the navigation lock clears, once per owner",()=>{
  assert.match(workspace,/useLayoutEffect\(\(\)=>\{/);
  assert.match(workspace,/if\(!id\)\{titleFocusOwner\.current=null;return;\}/);
  assert.match(workspace,/if\(locked\|\|preparation\.submitted\|\|titleFocusOwner\.current===id\)return;/);
  assert.match(workspace,/titleFocusOwner\.current=id;/);
  assert.match(workspace,/\},\[preparation\?\.id,preparation\?\.submitted,locked\]\);/);
  assert.match(workspace,/<input ref=\{titleField\} id="recording-title"/);
  assert.doesNotMatch(main,/requestAnimationFrame\(\(\)=>document\.getElementById\("recording-title"\)\?\.focus\(\)\)/);
});

test("delayed preparation focus never overrides an overlay or a chosen workspace control",()=>{
  assert.match(workspace,/!field\|\|field\.disabled\|\|overlayOpen\(\)\|\|/);
  assert.match(workspace,/active instanceof HTMLElement&&active!==field&&active\.closest\("\.recording-workspace"\)\)return;/);
  assert.match(workspace,/field\.focus\(\);/);
  assert.doesNotMatch(workspace,/setTimeout|requestAnimationFrame|\.select\(/);
});
