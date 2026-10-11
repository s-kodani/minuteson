import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
const panel = source("diarization-panel.tsx");

test("diarization owns one persistent settings boundary while its entry follows the selected tab", () => {
  assert.match(panel, /<OptionsModal className="speaker-diarization-tools" label="人数指定・話者分離" hidden=\{!visible\}/);
  assert.match(panel, /persistentReplacement=\{engineContent\} replacement=\{engineView\}/);
  assert.match(panel, /className="analysis-engine-view" hidden=\{!engineOpen\}/);
  assert.match(panel, /onDismiss:\(\)=>setEngineOpen\(false\),restoreTarget:\(\)=>engineTrigger.current/);
  assert.match(source("transcript.tsx"), /className="speaker-diarization-entry" ref=\{bindDiarizationHost\} hidden=\{!visible\}/);
  assert.match(source("analysis.tsx"), /visible=\{diarizationVisible\}/);
  assert.match(source("main.tsx"), /diarizationVisible=\{diarizationHost\?\.meetingId !== selectedMeeting.id \|\| detailTab === "transcript"\}/);
  assert.equal((panel.match(/<OptionsModal/g) ?? []).length, 1);
  assert.doesNotMatch(panel, /engineOpen && engineContent/);
});

test("workspace diarization shares the meeting native boundary without recreating its controller or fields", () => {
  const analysis = source("analysis.tsx"), main = source("main.tsx"), modal = source("common-modal.tsx");
  assert.match(panel, /if\(renderSettings\)/);
  assert.match(panel, /className="diarization-count-view" hidden=\{!countsOpen\|\|engineOpen\}/);
  assert.match(panel, /return renderSettings\(entry,view,body\)/);
  assert.match(panel, /parentKey:renderSettings\?"diarization-counts":undefined/);
  assert.match(analysis, /if \(renderTools\) return <DiarizationPanel/);
  assert.match(analysis, /\{createPortal\(diarizationEntry,diarizationMount\)\}/);
  assert.match(main, /open=\{meetingToolsOpen \|\| !!review\}/);
  assert.match(modal, /replacementParents.current.set\(previous.key/);
  assert.match(modal, /replacedView.current = returning\?\.view \?\?/);
  assert.match(modal, /\(usable \? returnTarget : cancel \?\? heading\)\?\.focus/);
});

test("diarization view changes retain one save queue, counts, save barriers and fixed job conditions", () => {
  assert.equal((panel.match(/new DraftSaveQueue/g) ?? []).length, 1);
  assert.match(panel, /registerPendingEdits\(\(\)=>queue.flush\(\)\)/);
  assert.match(panel, /const \[counts,setCounts\]=useState/);
  assert.match(panel, /withSavedEdits\(async\(\)=>/);
  assert.match(panel, /fixedDiarizationInput\(state,meetingId,"pending"\)/);
  assert.match(panel, /fixedTracks\(tracks,counts\)/);
  assert.match(panel, /validate: \(\) => JSON.stringify\(queue.snapshot\(\).payload\) === expected/);
  assert.doesNotMatch(panel, /useEffect\([^)]*setEngineOpen|setCounts\(\{\}\)/);
});

test("engine multiline fields share the rounded field and typography rules", () => {
  const css = source("prototype-alignment.css");
  assert.match(css, /\.analysis-engine-view :is\(input:not\(\[type=checkbox\]\),select,textarea\)[^\n]*border-radius:8px[^\n]*font-family:inherit[^\n]*font-size:13px/);
  assert.match(css, /\.analysis-engine-view textarea \{ min-height:80px; resize:vertical; \}/);
});

test("diarization browser regression uses visible controls and preserves the original full roundtrip", () => {
  const driver=readFileSync(new URL("./diarization-modal-browser-check.js",import.meta.url),"utf8");
  const runner=readFileSync(new URL("./run-browser-checks.mjs",import.meta.url),"utf8");
  assert.doesNotThrow(()=>new Function(`return (${driver})`));
  assert.match(runner,/\["diarization-modal", "browser-fixture.js", "diarization-modal-browser-check.js"\]/);
  assert.match(driver,/number.inputValue\(\) === "7"/);
  assert.match(driver,/dialog\[open\]/);
  assert.match(driver,/engineEntry.evaluate\(e => e === document.activeElement\)/);
  assert.doesNotMatch(driver,/invoke\(|monBrowserFixture|window\.|\.accept\(/);
});
