import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const analysis = readFileSync(new URL("../src/analysis.tsx", import.meta.url), "utf8");
const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

test("workspace analysis reviews reuse their owner boundary while the controller and original fields stay mounted", () => {
  assert.match(analysis, /renderTools\?: \(tools: ReactNode, review: ModalReplacement \| null, persistentViews: ReactNode\) => ReactNode/);
  assert.match(analysis, /key: `\$\{preview.analysis.id\}:\$\{preview.restoring\}`/);
  assert.match(analysis, /labelledBy: "analysis-preview-title", className: "analysis-dialog", content: reviewContent/);
  assert.match(analysis, /blocked: disabled, onDismiss: \(\) => setPreview\(null\), restoreTarget: \(\) => returnFocus.current/);
  assert.match(analysis, /renderEditor=\{\(entry, vocabularyView, vocabularyBody\)=>renderTools\(tools\(entry\), visibleView \?\? diarizationView \?\? vocabularyView, <>\{persistentViews\}\{diarizationBody\}\{vocabularyBody\}<\/>\)\}/);
  assert.ok(analysis.indexOf("if (renderTools) return") < analysis.indexOf("return <>{tools("));
  assert.match(main, /replacement=\{review\}/);
  assert.match(main, /<div className="meeting-analysis-tools">\{analysisTools\}<\/div>/);
  assert.match(main, /<AnalysisPanel key=\{selectedMeeting.id\}[\s\S]*renderTools=\{renderMeetingTools\}/);
  assert.match(main, /renderMeetingTools\(null, null\)/);
});

test("workspace engine settings keep a persistent body in the same native boundary with explicit entry and return focus", () => {
  const options = readFileSync(new URL("../src/options-modal.tsx", import.meta.url), "utf8");
  assert.match(options, /persistentReplacement=\{persistentReplacement\}/);
  assert.match(main, /persistentReplacement=\{persistentViews\}/);
  assert.match(analysis, /className="analysis-engine-view" hidden=\{!engineOpen \|\| !!preview\}/);
  assert.match(analysis, /<div className="options-dialog-content">\{engineContent\}<\/div>/);
  assert.match(analysis, /const visibleView = review \?\? \(engineOpen \? \{/);
  assert.match(analysis, /onDismiss: \(\) => setEngineOpen\(false\), restoreTarget: \(\) => engineTrigger.current/);
  assert.match(analysis, /event.currentTarget.focus\(\{preventScroll:true\}\);setEngineOpen\(true\);/);
  assert.match(analysis, /: <OptionsModal className="analysis-settings"[^\n]*\{engineContent\}/);
  assert.doesNotMatch(analysis, /engineOpen && engineContent|useEffect\([^)]*setEngineOpen/);
  assert.match(analysis, /\(\) => new DraftSaveQueue<AnalysisPreferences, Preferences>/);
  assert.match(analysis, /registerPendingEdits\(\(\) => queue.flush\(\)\)/);
});

test("analysis review presentation retains explicit confirmation, save barrier, version and receipt verification", () => {
  assert.match(analysis, /assertReviewedVersion\(state, summary, restoring\)/);
  assert.match(analysis, /assertReviewedVersion\(state, preview.analysis, preview.restoring\)/);
  assert.match(analysis, /await confirm\(\{ opener: confirmationOpener/);
  assert.match(analysis, /action: \(\) => saved\(async \(\) =>/);
  assert.match(analysis, /await publication.apply\([\s\S]*"adopt_analysis_candidate"/);
  assert.match(analysis, /expectedMeetingLockVersion: state.meetingLockVersion, expectedDraft: state.draft/);
  assert.match(analysis, /disabled=\{disabled \|\| publication.inFlight\}/);
  assert.match(analysis, /\{publication.status\}/);
  assert.doesNotMatch(analysis, /renderTools\([\s\S]*invoke\(/);
});

test("analysis review regression distinguishes returning to the owner from dismissing the whole modal", () => {
  const driver = readFileSync(new URL("./document-modal-browser-check.js", import.meta.url), "utf8");
  assert.doesNotThrow(() => new Function(`return (${driver})`));
  assert.match(driver, /parentName = null/);
  assert.match(driver, /await modal\(parentName\).waitFor\(\)/);
  assert.match(driver, /cancelled\("候補版の採用確認 #2", review, analysisOwner\)/);
  assert.match(driver, /removed review opener did not return to owner heading/);
  assert.doesNotMatch(driver, /document.activeElement\?\.textContent === "ローカル文字起こし"/);
});

test("engine settings regression traverses owner, settings, preview and confirmation through visible controls", () => {
  const driver = readFileSync(new URL("./analysis-engine-modal-browser-check.js", import.meta.url), "utf8");
  const runner = readFileSync(new URL("./run-browser-checks.mjs", import.meta.url), "utf8");
  assert.doesNotThrow(() => new Function(`return (${driver})`));
  assert.match(runner, /\["analysis-engine-modal", "browser-fixture.js", "analysis-engine-modal-browser-check.js"\]/);
  assert.match(driver, /threads.inputValue\(\) === "6"/);
  assert.match(driver, /page.locator\("dialog\[open\]"\).count\(\) === 1/);
  assert.match(driver, /name: "確認をキャンセル"/);
  assert.doesNotMatch(driver, /invoke\(|monBrowserFixture|detailUI\.|window\.|\.accept\(/);
});
