import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");

test("import vocabulary shares its native boundary and preserves fields while inactive",()=>{
  const panel=source("audio-import.tsx"), modal=source("common-modal.tsx");
  assert.equal((panel.match(/<ScopedModal\b/g)??[]).length,1);
  assert.doesNotMatch(panel,/<OptionsModal|vocabularyOpen && vocabularyContent/);
  assert.match(panel,/persistentReplacement=\{vocabularyContent\} replacement=\{vocabularyOpen \?/);
  assert.match(modal,/\(replacement \|\| persistentReplacement != null\) && <div className="common-modal-replacement" hidden=\{!replacement \|\| !!owned\}>\{persistentReplacement \?\? replacement\?\.content\}/);
  assert.match(modal,/ref=\{baseBody\} className="common-modal-body" hidden=\{!!owned \|\| !!replacement\}>\{children\}/);
});

test("vocabulary switch restores its own entry and retains original save and confirmation guards",()=>{
  const panel=source("audio-import.tsx");
  assert.match(panel,/id="audio-import-dialog"/);
  assert.match(panel,/aria-controls=\{scope\?\.id\?\?"audio-import-dialog"\} aria-expanded=\{vocabularyOpen\}/);
  assert.match(panel,/event.currentTarget.focus\(\{preventScroll:true\}\);setVocabularyOpen\(true\)/);
  assert.match(panel,/onDismiss:\(\)=>setVocabularyOpen\(false\),restoreTarget:\(\)=>vocabularyTrigger.current/);
  assert.match(panel,/className="import-vocabulary-fields" disabled=\{disabled\|\|!!snapshot\?\.saved\}/);
  assert.match(panel,/setVocabularyOpen\(false\);setOpen\(true\)/);
  assert.match(panel,/JSON.stringify\(inputRef.current\) === expected && snapshotRef.current\?\.token === token && !snapshotRef.current\?\.busy && !snapshotRef.current\?\.saved/);
  assert.match(panel,/canStartImport\(snapshot,title,terms,common\?\.lockVersion\?\?null,recordingActive\)/);
  assert.match(panel,/terms.length>=500/);
  assert.match(panel,/terms.slice\(index\*50,\(index\+1\)\*50\)/);
  assert.match(panel,/onClick=\{event=>removeTerm\(entry,event.currentTarget\)\}/);
  assert.match(panel,/void confirm\(\{ opener:actionOpener, title: "取り込み用語の削除を確認"/);
});

test("persistent vocabulary fields use a single column without losing narrow wrapping",()=>{
  const css=source("prototype-alignment.css");
  assert.match(css,/\.import-dialog fieldset\.import-vocabulary-fields \{ grid-template-columns:minmax\(0,1fr\); \}/);
  assert.match(css,/\.import-vocabulary-fields \.import-term :is\(input:not\(\[type=checkbox\]\),textarea\) \{[^}]*padding:9px 12px; border:1px solid var\(--input-border\);[^}]*font-size:13px; line-height:20px;/);
  assert.match(css,/\.import-vocabulary-fields \.import-term input:not\(\[type=checkbox\]\) \{ min-height:40px; \}/);
  assert.match(css,/\.import-dialog \.import-vocabulary-entry \{ display:flex; align-items:center; flex-wrap:wrap; gap:8px;/);
});
