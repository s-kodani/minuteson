import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const view = readFileSync(new URL("../src/vocabulary.tsx", import.meta.url), "utf8");
const analysis = readFileSync(new URL("../src/analysis.tsx", import.meta.url), "utf8");

test("meeting vocabulary keeps one controller and a persistent editor in its workspace owner", () => {
  assert.match(view, /renderEditor\?: \(entry: ReactNode, view: ModalReplacement \| null, persistentEditor: ReactNode\) => ReactNode/);
  assert.match(view, /if \(meetingId && renderEditor\)/);
  assert.match(view, /className="meeting-vocabulary-view" hidden=\{!open\}/);
  assert.match(view, /return renderEditor\(entry, view, persistentEditor\)/);
  assert.match(view, /onDismiss:\(\)=>setOpen\(false\), restoreTarget:\(\)=>trigger.current/);
  assert.match(view, /event.currentTarget.focus\(\{preventScroll:true\}\);setOpen\(true\);/);
  assert.match(view, /return meetingId \? <section>/);
  assert.match(analysis, /if \(renderTools\) return <DiarizationPanel/);
  assert.match(analysis, /renderTools\(tools\(entry\), visibleView \?\? diarizationView \?\? vocabularyView, <>\{persistentViews\}\{diarizationBody\}\{vocabularyBody\}<\/>\)/);
  assert.match(analysis, /const tools = \(vocabularyEntry: ReactNode\)/);
  assert.match(analysis, /\{vocabularyEntry\}/);
});

test("vocabulary presentation preserves persistence and explicit destructive decisions without duplicating notification effects", () => {
  assert.match(view, /registerPendingEdits\(async \(\) => \{\s*await queue.flush\(\)/);
  assert.match(view, /if \(loading \|\| confirm.pending \|\| !state.dirty/);
  assert.match(view, /validate: \(\) => JSON.stringify\(queue.snapshot\(\).payload\) === expected/);
  assert.match(view, /action: async \(\) => \{ queue.edit\(value => value.filter/);
  assert.match(view, /if \(validVocabulary\(queue.snapshot\(\).payload \?\? \[\]\)\) await queue.flush\(\)/);
  const persistent = view.slice(view.indexOf("const persistentEditor"), view.indexOf("const view: ModalReplacement"));
  assert.match(persistent, /<IssueStatus/);
  assert.doesNotMatch(persistent, /\{issue\}|<IssueNotice/);
  assert.equal((view.match(/new DraftSaveQueue/g) ?? []).length, 1);
});
