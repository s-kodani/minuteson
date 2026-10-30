import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { flushPendingEdits, registerPendingEdits } from "../src/draft-save.ts";
import { withConfirmedEdits } from "../src/confirmed-action.ts";
const source=(file:string)=>readFileSync(new URL(`../src/${file}`,import.meta.url),"utf8");

test("transcription publication waits for an open edit and propagates failed save without publication",async()=>{
  const events:string[]=[];
  const unregister=registerPendingEdits(async reason=>{events.push(reason);throw Error("save failed");});
  try {
    await assert.rejects(withConfirmedEdits(async action=>{try{await flushPendingEdits("transcription");await action();}catch{/* Main window reports failure. */}},async()=>{events.push("publish");}),/編集の保存/);
    assert.deepEqual(events,["transcription"]);
  } finally { unregister(); }
});
test("a successful edit barrier runs one publication after the saved input",async()=>{
  const events:string[]=[];
  const unregister=registerPendingEdits(async reason=>{events.push(`saved:${reason}`);});
  try {
    await withConfirmedEdits(async action=>{await flushPendingEdits("transcription");await action();},async()=>{events.push("publish");});
    assert.deepEqual(events,["saved:transcription","publish"]);
  } finally { unregister(); }
});
test("regeneration is explicit, retains unknown acceptance identity and never requests meeting model overrides",()=>{
  const view=source("analysis.tsx");
  assert.match(view,/title: initial \? "文字起こしを生成" : "文字起こしを再生成"/);
  assert.match(view,/get_transcription_regeneration_target/);
  assert.match(view,/regeneration.current = request/);
  assert.match(view,/saved\?\.id === request.operationId/);
  assert.match(view,/Unknown acceptance: retain the same intent/);
  assert.doesNotMatch(view,/executablePath|modelPath|save_analysis_preferences/);
});
test("publication is coordinated outside the selected meeting and does not save or discard Markdown",()=>{
  const hook=source("analysis-publication.ts"),main=source("main.tsx")+source("saved-navigation.ts"),minutes=source("minutes.tsx");
  assert.match(hook,/get_ready_analysis_flows/);
  assert.match(hook,/publishWithReplacementConfirmation/);
  assert.match(hook,/expectedLockVersion: current.lockVersion, confirmedTarget/);
  assert.match(main,/useAnalysisPublication\(writeLocked, action => withSavedEdits\(action, "transcription"\)\)/);
  assert.match(main,/reason === "transcription"\) flushSync/);
  assert.match(minutes,/if \(reason === "transcription"\) return/);
});
