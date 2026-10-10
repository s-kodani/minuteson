import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
test("meeting title precedes tabs and all fixed chrome participates in dock sizing", () => {
  const main = source("main.tsx");
  assert.ok(main.indexOf('{detail && <div className="meeting-detail-status"') < main.indexOf('{detail && <div className="meeting-detail-tabs"'));
  assert.match(main, /heading\?\.getBoundingClientRect\(\)\.height/);
  assert.match(main, /querySelectorAll<HTMLElement>\("\.meeting-detail-status,\.meeting-detail-tabs"\)/);
});
test("transcript primary reading keeps guarded text and time actions while advanced inputs stay mounted", () => {
  const transcript = source("transcript.tsx");
  assert.match(transcript, /className="transcript-body-edit"[^\n]*disabled=\{!editable\} onClick=\{\(\) => beginTextEdit\(utterance\)\}/);
  assert.match(transcript, /className="utterance-stamp"[^>]*onClick=\{\(\) => void play\(utterance\)\}/);
  for (const section of ["speaker-name-tools", "speaker-merge-tools", "utterance-edit-tools"]) assert.match(transcript, new RegExp(`<details className="${section}"`));
  assert.match(transcript, /<CommonModal ref=\{splitDialog\}/);
  assert.match(transcript, /<IssueNotice scope=\{meetingId\} type="transcript-save"/);
});
test("minutes edit actions use the existing editor owner and generation stays guarded", () => {
  const panel = source("minutes.tsx"), editor = source("minutes-editor.tsx");
  assert.match(panel, /actionsHost=\{editorActionsHost\}/);
  assert.match(editor, /actionsHost \? createPortal\(primaryActions, actionsHost\) : primaryActions/);
  assert.match(editor, /await queue\.flush\(\);[\s\S]*setEditing\(false\)/);
  assert.match(editor, /focused === document\.activeElement/);
  assert.match(editor, /target\.focus\(\{ preventScroll: true \}\)/);
  assert.match(panel, /recordingActive \|\| pending \|\| !source\?\.currentId \|\| !engine \|\| !minutesConfigured\(engine\)/);
  assert.match(panel, /className="minutes-empty-state"/);
  const evidence = source("minutes-evidence.tsx");
  assert.match(evidence, /<InfoPopover label="根拠の確認"/);
  assert.match(evidence, /<details className="evidence-record-details"/);
  assert.match(evidence, /根拠の音声を再生/);
});
