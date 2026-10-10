import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { installMinutesFixture } from "./minutes-browser-fixture.js";
import { applyMinutesFixtureTheme } from "./minutes-browser-helpers.js";

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

test("expanded idle playback settings keep a bounded scrollport in the minimum desktop frame", () => {
  const css=readFileSync(new URL("../src/prototype-alignment.css",import.meta.url),"utf8");
  assert.match(css,/@media\(max-width:1119px\) and \(max-height:760px\)/);
  assert.match(css,/\.meeting-playback-dock:has\(\.meeting-audio-details\[open\]\):not\(:has\(\.audio-player\)\) \{ max-height:80px; align-items:start; scroll-padding-block:8px;/);
  assert.doesNotMatch(css,/\.minutes-primary-toolbar \{[^}]*position:sticky/);
  assert.match(css,/\.minutes-format-tools:not\(\[open\]\) \{ padding:0; border:0; background:transparent;/);
  const audio=readFileSync(new URL("../src/meeting-audio.tsx",import.meta.url),"utf8");
  assert.match(audio,/<TextSizeSelection locked=\{locked\} \/>/);
  assert.match(audio,/<PlaybackFollowingSelection locked=\{locked\} \/>/);
});
