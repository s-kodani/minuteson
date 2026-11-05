import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { applyPlaybackFixtureTheme } from "./playback-browser-helpers.js";

test("playback fixture theme events preserve stored preferences and use the production schema", async () => {
  const preferences={id:"ui_preferences",payload:{schemaVersion:1,theme:"system",textSize:20},lockVersion:7};
  let events=0, waited="";
  const fixture={uiPreferences:preferences,emitUiPreferences:async()=>{events++;}};
  const page={
    evaluate:async(run:Function,arg:string)=>{
      // Execute the fixture adapter in an isolated scope, not in a browser.
      const evaluate=Function("window","theme",`return (${run.toString()})(theme);`);
      await evaluate({monBrowserFixture:fixture},arg);
    },
    waitForFunction:async(_run:Function,theme:string)=>{waited=theme;},
  };
  await applyPlaybackFixtureTheme(page,"dark");
  assert.equal(fixture.uiPreferences,preferences);
  assert.deepEqual(preferences.payload,{schemaVersion:1,theme:"dark",textSize:20});
  assert.equal(preferences.lockVersion,8);
  assert.equal(events,1);
  assert.equal(waited,"dark");
  await assert.rejects(()=>applyPlaybackFixtureTheme(page,"sepia"),/invalid synthetic theme/);
  assert.equal(events,1);
});

test("playback fixture starts missing preferences with a valid schema and refuses a foreign fixture", async () => {
  const fixture:{uiPreferences?:unknown;emitUiPreferences?:()=>Promise<void>}={emitUiPreferences:async()=>{}};
  const page={evaluate:async(run:Function,arg:string)=>Function("window","theme",`return (${run.toString()})(theme);`)({monBrowserFixture:fixture},arg),waitForFunction:async()=>{}};
  await applyPlaybackFixtureTheme(page,"light");
  assert.deepEqual(fixture.uiPreferences,{id:"ui_preferences",payload:{schemaVersion:1,theme:"light",colorTheme:"monochrome"},lockVersion:1});
  delete fixture.emitUiPreferences;
  await assert.rejects(()=>applyPlaybackFixtureTheme(page,"dark"),/playback fixture is required/);
});

test("cancellation driver uses the initial bar without restoring deleted header controls", () => {
  const driver=readFileSync(new URL("./playback-cancellation-browser-check.js",import.meta.url),"utf8");
  const runner=readFileSync(new URL("./browser/regression.spec.mjs",import.meta.url),"utf8");
  assert.doesNotMatch(driver,/会議全体を再生|表示テーマ|編集を開始|getByText\("再生トラック"/);
  assert.match(driver,/const start=player.getByRole\("button",\{name:"再生",exact:true\}\)/);
  assert.match(driver,/detailUI.applyPlaybackTheme\(theme\)/);
  assert.match(runner,/applyPlaybackTheme: theme => applyPlaybackFixtureTheme\(page, theme\)/);
  assert.match(driver,/register_playback_preparation"\)===0/);
});

test("manual playback fixture exposes protocol faults and ownership without changing active reservations", () => {
  const fixture=readFileSync(new URL("./detail-mock-browser-fixture.js",import.meta.url),"utf8");
  assert.match(fixture,/音声準備プロトコル（合成）/);
  for(const scenario of ["register","prepareLost","receiptLost","responsive","strict"]) assert.ok(fixture.includes(`protocol.value===\"${scenario}\"`) || fixture.includes(`[\"${scenario}\",`));
  assert.match(fixture,/if\(Object.keys\(f.preparation.reservations\).length\|\|Object.keys\(f.meetingAudio.sessions\).length\)return/);
  assert.match(fixture,/準備所有: \$\{Object.keys\(f.preparation.reservations\).length\}/);
  assert.match(fixture,/音声所有: \$\{Object.keys\(f.meetingAudio.sessions\).length\}/);
  assert.match(fixture,/protocol.disabled=Object.keys\(f.preparation.reservations\).length>0\|\|Object.keys\(f.meetingAudio.sessions\).length>0/);
});
