import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { installMinutesFixture } from "./minutes-browser-fixture.js";
import { applyMinutesFixtureTheme, minutesFixtureIntegrity } from "./minutes-browser-helpers.js";
import { assertPublicationReceipt, publicationFromHead, type PublicationReview } from "../src/version-publication-state.ts";

test("synthetic publication faults require read-only receipt verification without replaying adoption or restore", async () => {
  for (const restoring of [false,true]) for (const mode of ["lost","foreign"]) {
    let initialize: (() => void) | undefined;
    await installMinutesFixture({addInitScript:async(callback:()=>void)=>{initialize=callback;},reload:async()=>{}});
    const fakeWindow:any={};
    runInNewContext(`(${initialize!.toString()})()`,{window:fakeWindow,structuredClone});
    const data=fakeWindow.__mon, invoke=fakeWindow.__TAURI_INTERNALS__.invoke;
    const original=JSON.stringify({draft:data.drafts.a,generated:data.views.a.content});
    if (!restoring) {
      await invoke("start_minutes_generation",{request:{input:{operationId:"test-generation",expectedDraft:{lockVersion:data.td.lockVersion},expectedMeetingLockVersion:data.lock}}});
      data.complete();
    }
    const head=await invoke("get_minutes_versions",{meetingId:"m1"});
    const id=restoring?"p":"b", version=data.versions.find((v:any)=>v.id===id);
    const review:PublicationReview={meetingId:"m1",kind:"minutes",id,contentHash:version.contentHash,revisionNo:version.revisionNo,meetingLockVersion:head.meetingLockVersion,currentId:head.currentId,restoring};
    const command=restoring?"restore_minutes_version":"adopt_minutes_candidate";
    const request={meetingId:"m1",expectedMeetingLockVersion:head.meetingLockVersion,expectedDraft:head.draft,expectedContentHash:review.contentHash,...(restoring?{expectedPreviousId:id}:{revisionId:id})};
    data.lostPublicationResponse=mode==="lost"; data.foreignPublicationReceipt=mode==="foreign";
    if (mode==="lost") await assert.rejects(()=>invoke(command,restoring?request:{request}),/応答喪失/);
    if (mode==="foreign") {
      const receipt=await invoke(command,restoring?request:{request});
      assert.equal(receipt.id,"foreign"); assert.throws(()=>assertPublicationReceipt(review,receipt));
    }
    const verified=publicationFromHead(review,await invoke("get_minutes_versions",{meetingId:"m1"}));
    assert.equal(verified.id,id); assert.equal(data.current,id); assert.equal(data.previous,"a");
    assert.equal(data.calls.filter((call:any)=>call.cmd===command).length,1);
    assert.equal(JSON.stringify({draft:data.drafts.a,generated:data.views.a.content}),original);
  }
});

test("publication and notification regressions explicitly cross modal boundaries before outside actions", () => {
  for (const file of ["minutes-modal-browser-check.js","minutes-toast-browser-check.js","minutes-publication-toast-browser-check.js"]) {
    const code=readFileSync(new URL(file,import.meta.url),"utf8");
    assert.doesNotThrow(()=>new Function(`return (${code})`));
    assert.doesNotMatch(code,/detailUI\.openMinutes\(/);
    assert.match(code,/getByRole\("tab", \{ name: "議事録", exact: true \}\)/);
    assert.match(code,/getByRole\("button", \{ name: "議事録の設定・生成状況・候補", exact: true \}\)/);
  }
  const publication=readFileSync(new URL("minutes-publication-toast-browser-check.js",import.meta.url),"utf8");
  assert.match(publication,/await closeSettings\(\); await panel\(\)\.getByRole\("button", \{ name: "採用・復元の結果だけを確認"/);
  assert.match(publication,/await closeSettings\(\); await host\(\)\.getByRole\("button", \{ name: "エラー通知を閉じる"/);
  assert.doesNotMatch(publication,/getByText\("議事録エンジン設定（端末内に保存）"[^\n]*\.click\(/);
});

test("minutes integrity readout detects same-size text/evidence changes and exact undo independently of generated content", async () => {
  let initialize: (() => void) | undefined;
  await installMinutesFixture({ addInitScript: async (callback: () => void) => { initialize=callback; }, reload:async () => {} });
  const fakeWindow: any = {};
  runInNewContext(`(${initialize!.toString()})()`, { window:fakeWindow,structuredClone });
  const data=fakeWindow.__mon;
  const original=structuredClone(data.drafts.a.payload);
  const baseline={draft:JSON.stringify(original),generated:JSON.stringify(data.views.a.content)};
  const before=minutesFixtureIntegrity(data,baseline);
  assert.equal(before.originalDraft,true); assert.equal(before.originalGenerated,true);
  data.drafts.a.payload.blocks[0].content[0].text = "changed heading";
  assert.equal(minutesFixtureIntegrity(data,baseline).originalDraft,false);
  assert.equal(minutesFixtureIntegrity(data,baseline).blocks,before.blocks);
  data.drafts.a.payload=structuredClone(original);
  data.drafts.a.payload.evidence[0].quoteText = "changed evidence";
  assert.equal(minutesFixtureIntegrity(data,baseline).originalDraft,false);
  assert.equal(minutesFixtureIntegrity(data,baseline).evidence,before.evidence);
  data.drafts.a.payload=structuredClone(original);
  data.calls.push({cmd:"undo_minutes_edit"});
  const restored=minutesFixtureIntegrity(data,baseline);
  assert.equal(restored.originalDraft,true);assert.equal(restored.originalGenerated,true);assert.equal(restored.undoRequests,1);
  data.views.a.content={changed:true};
  assert.equal(minutesFixtureIntegrity(data,baseline).originalDraft,true);
  assert.equal(minutesFixtureIntegrity(data,baseline).originalGenerated,false);
});

test("external synthetic theme commits preserve reading fields and minutes, then emit the real hint", async () => {
  let initialize: (() => void) | undefined;
  await installMinutesFixture({ addInitScript: async (callback: () => void) => { initialize=callback; }, reload:async () => {} });
  const fakeWindow: any = {};
  runInNewContext(`(${initialize!.toString()})()`, { window:fakeWindow,structuredClone });
  const data=fakeWindow.__mon, hints:any[]=[];
  const handler=fakeWindow.__TAURI_INTERNALS__.transformCallback((event:any)=>hints.push(event));
  await fakeWindow.__TAURI_INTERNALS__.invoke("plugin:event|listen",{event:"ui-preferences-changed",handler});
  data.ui.payload={schemaVersion:2,theme:"light",textSize:20,followPlayback:true};
  const before=JSON.stringify({drafts:data.drafts,views:data.views,calls:data.calls});
  data.changeThemeFromOtherWindow("dark");
  assert.equal(JSON.stringify(data.ui.payload),JSON.stringify({schemaVersion:2,theme:"dark",textSize:20,followPlayback:true}));
  assert.equal(data.ui.lockVersion,1);assert.equal(hints.length,1);
  assert.equal(hints[0].event,"ui-preferences-changed");assert.equal(hints[0].payload,1);
  assert.equal(JSON.stringify({drafts:data.drafts,views:data.views,calls:data.calls}),before);
  const confirmed=JSON.stringify(data.ui);
  assert.throws(()=>data.changeThemeFromOtherWindow("unknown"));
  assert.equal(JSON.stringify(data.ui),confirmed);assert.equal(hints.length,1);
});

test("theme regression helper rejects invalid values and waits for confirmed provider colors", async () => {
  const calls:any[]=[];
  const page={evaluate:async (fn:Function,value:string)=>{calls.push(["commit",value,fn.toString()]);},waitForFunction:async(fn:Function,value:string)=>{calls.push(["confirmed",value,fn.toString()]);}};
  await assert.rejects(()=>applyMinutesFixtureTheme(page,"unknown"));assert.equal(calls.length,0);
  await applyMinutesFixtureTheme(page,"dark");
  assert.deepEqual(calls.map(([kind,value])=>[kind,value]),[["commit","dark"],["confirmed","dark"]]);
  assert.match(calls[0][2],/changeThemeFromOtherWindow/);assert.match(calls[1][2],/documentElement.dataset.theme === theme/);
});

test("minutes regression drivers use explicit panel, primary actions and rich evidence without a header theme", () => {
  const read=(file:string)=>readFileSync(new URL(file,import.meta.url),"utf8");
  for(const file of ["minutes-panel-browser-check.js","detail-minutes-browser-check.js","display-minutes-browser-check.js"]){
    const code=read(file);assert.match(code,/detailUI.applyMinutesTheme/);assert.doesNotMatch(code,/getByRole\("banner"\).*combobox/);
    assert.doesNotThrow(()=>new Function(`return (${code})`));
  }
  const panel=read("minutes-panel-browser-check.js");
  assert.match(panel,/const goHeading=async\(label,index\)=>\{await openOutline\(\)/);
  assert.match(panel,/await rich.isVisible\(\)\?rich:doc\(\)/);
  assert.doesNotMatch(panel,/editor\(\).getByRole\("button",\{name:"(?:議事録を編集|編集を終了|議事録保存を再試行)"/);
  assert.match(panel,/document.activeElement===window.referenceOpener/);
  assert.match(read("run-browser-checks.mjs"),/applyMinutesFixtureTheme.toString\(\)/);
});

test("minutes panel regression measures available paper width and aligned opener, not the old cap", () => {
  const code=readFileSync(new URL("./minutes-panel-browser-check.js",import.meta.url),"utf8");
  assert.doesNotThrow(()=>new Function(`return (${code})`));
  assert.match(code,/Math.abs\(closed.paperWidth-closed.available\)<1&&closed.rightGap<1/);
  assert.match(code,/Math.abs\(bounds.document-available\)<1/);
  assert.doesNotMatch(code,/bounds.document<=760|max-760px/);
});

test("shared playback stays a compact row and settings use an explicit modal", () => {
  const css=readFileSync(new URL("../src/prototype-alignment.css",import.meta.url),"utf8");
  assert.match(css,/\.meeting-playback-dock \{ display:grid;[^}]*align-items:start;[^}]*max-height:none; overflow:visible;/);
  assert.doesNotMatch(css,/\.minutes-primary-toolbar \{[^}]*position:sticky/);
  assert.match(css,/\.minutes-format-tools:not\(\[open\]\) \{ padding:0; border:0; background:transparent;/);
  const audio=readFileSync(new URL("../src/meeting-audio.tsx",import.meta.url),"utf8");
  assert.match(audio,/<TextSizeSelection locked=\{locked\} \/>/);
  assert.match(audio,/<PlaybackFollowingSelection locked=\{locked\} \/>/);
  assert.match(audio,/<OptionsModal label="再生設定" icon="more" className="meeting-audio-settings"/);
  assert.doesNotMatch(audio,/<details className="meeting-audio-details"/);
});

test("minutes edit confirmation regression follows modal settings and captures selection before opening format", () => {
  const code=readFileSync(new URL("./minutes-edit-confirmation-browser-check.js",import.meta.url),"utf8");
  assert.doesNotThrow(()=>new Function(`return (${code})`));
  assert.match(code,/await item\.click\(\); await openFormat\(\)/);
  assert.match(code,/await closeFormat\(\);/);
  assert.match(code,/await openManagement\(\); await management\(\)\.getByRole/);
  assert.match(code,/await closeManagement\(\);/);
  assert.doesNotMatch(code,/editor\(\)\.getByRole\("button", \{ name: "(?:議事録を編集|議事録保存を再試行|議事録編集を再読込|直前の議事録編集を取り消す)"/);
  assert.doesNotMatch(code,/detailUI\.openMinutes\(|getByText\("議事録の書式・構造操作"/);
  assert.match(code,/Cancellation resumes ordinary autosave/);
  assert.match(code,/await count\("save_minutes_draft"\) === 1, "discard saved unwanted rich input or replayed failed save"/);
});

test("minutes workspace regression covers failed exit, explicit retries and surviving focus without extra saves",()=>{
  const code=readFileSync(new URL("./detail-minutes-browser-check.js",import.meta.url),"utf8");
  assert.doesNotThrow(()=>new Function(`return (${code})`));
  assert.match(code,/failed exit silently replayed a save or exited editing/);
  assert.match(code,/failed explicit retry lost focus\/input or sent multiple saves/);
  assert.match(code,/document.activeElement\?\.textContent === "議事録保存を再試行"/);
  assert.match(code,/document.activeElement\?\.getAttribute\("aria-label"\) === "編集を終了"/);
  assert.match(code,/document.activeElement\?\.getAttribute\("aria-label"\) === "議事録を編集"/);
  assert.match(code,/clean exit unnecessarily saved again/);
});

test("synthetic controls can be dismissed without clearing the failure or changing product controls", () => {
  const fixture=readFileSync(new URL("./detail-mock-browser-fixture.js",import.meta.url),"utf8");
  assert.match(fixture,/close.textContent = "合成操作を閉じる"/);
  assert.match(fixture,/close.onclick = \(\) => \{ controls.hidden = true; \}/);
  const css=readFileSync(new URL("../src/prototype-alignment.css",import.meta.url),"utf8");
  assert.match(css,/\.meeting-playback-dock \{[^}]*overflow:visible;/);
  const player=readFileSync(new URL("../src/transcript-audio.tsx",import.meta.url),"utf8");
  for (const action of ["先頭へ","再生を閉じる","会議全体の再生位置","速度"]) assert.ok(player.includes(action));
});
