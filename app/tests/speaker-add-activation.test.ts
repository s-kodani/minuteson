import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const view=readFileSync(new URL("../src/transcript.tsx",import.meta.url),"utf8");
const css=readFileSync(new URL("../src/prototype-alignment.css",import.meta.url),"utf8");

test("speaker add begins the existing draft and recovery path before opening, without publishing a speaker",()=>{
  assert.match(view,/kind\?:"name"\|"add"/);
  assert.match(view,/pendingTextOpen.current=\{analysisId:analysis.id,id:utteranceId\?\?"",opener,kind:"add"\}/);
  assert.match(view,/if\(!canBeginText\|\|!analysis\)return/);
  assert.match(view,/if \(!editable \|\| !view\) return/);
  assert.match(view,/document.activeElement !== pending.opener && document.activeElement !== document.body/);
  assert.match(view,/if \(pending.kind === "add"\)/);
  assert.match(view,/view.utterances.some\(row=>row.id===pending.id\)/);
  const request=view.slice(view.indexOf("const requestSpeakerAdd"),view.indexOf("const openNameEditor"));
  assert.match(request,/void begin\(\)/);
  assert.doesNotMatch(request,/addSpeaker\(|saveQueue.edit|structuralEdit\(/);
});

test("speaker list and utterance add share safe activation and responsive opener restoration",()=>{
  assert.match(view,/onClick=\{event=>requestSpeakerAdd\(event.currentTarget\)\}/);
  assert.match(view,/requestSpeakerAdd\(event.currentTarget,utterance.id\)/);
  assert.match(view,/if\(pendingTextOpen.current\|\|adding\|\|!view\|\|view.speakers.length>=100\)return/);
  assert.match(view,/speakerPanel.current\?\.dismiss\(\)/);
  assert.match(view,/restoreTarget=\{\(\)=>addOpener.current\?\.getClientRects\(\).length\?addOpener.current/);
  assert.match(view,/\}>作成して保存<\/button><button type="button" className="small-secondary"/);
  assert.match(css,/\.common-modal.speaker-add-dialog \{ width:min\(520px,calc\(100vw - 32px\)\); max-width:520px;/);
  assert.match(css,/\.speaker-add-dialog>\.common-modal-body:not\(\[hidden\]\) \{ display:grid; gap:16px/);
});
