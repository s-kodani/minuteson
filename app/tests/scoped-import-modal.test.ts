import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { SettingsModalState, type SettingsView } from "../src/settings-modal-state.ts";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");

test("busy import owns the shared boundary until its controller closes it", () => {
  const state = new SettingsModalState(); let calls = 0;
  const imported: SettingsView = {id:"import",open:true,title:"音声を取り込む",className:"",dialogClassName:"import-dialog",
    labelledBy:"import-title",blocked:true,restoreReady:false,dismissLabel:"キャンセル",dismiss:()=>{calls++;},entry:()=>null};
  state.update(imported);
  assert.equal(state.getSnapshot().active, imported);
  assert.equal(state.getSnapshot().active?.blocked, true);
  assert.equal(state.getSnapshot().active?.dismissLabel, "キャンセル");
  assert.equal(calls, 0);
  state.update({...imported,open:false,blocked:false,restoreReady:true});
  assert.equal(state.getSnapshot().active, null);
  assert.equal(state.getView("import")?.restoreReady, true);
  state.update({id:"tools",open:true,title:"会議操作",className:"",dismiss:()=>{},entry:()=>null});
  assert.equal(state.getSnapshot().active?.id, "tools");
  assert.equal(state.getSnapshot().active?.blocked, undefined);
  assert.equal(calls, 0);
});

test("a closed retained import descriptor receives readiness without reopening it", () => {
  const state = new SettingsModalState();
  const view: SettingsView = {id:"import",open:true,title:"取り込み",className:"",restoreReady:false,dismiss:()=>{},entry:()=>null};
  state.update(view); state.update({...view,open:false});
  state.update({...view,open:false,restoreReady:true});
  assert.equal(state.getSnapshot().active, null);
  assert.equal(state.getView(view.id)?.restoreReady, true);
  state.remove(view.id); assert.equal(state.getView(view.id), null);
});

test("import and vocabulary use the same persistent native boundary as meeting settings", () => {
  const importPanel = source("audio-import.tsx"), modal = source("scoped-modal.tsx"), scope = source("settings-modal-scope.tsx");
  assert.match(importPanel, /<ScopedModal title="音声を取り込む"/);
  assert.match(importPanel, /dialog=scope\?\.dialog\?\?ownDialog/);
  assert.match(modal, /scope.body && createPortal/);
  assert.match(modal, /scope.replacement && createPortal/);
  assert.match(modal, /hidden=\{hidden\}/);
  assert.match(modal, /if \(!scope\) return <CommonModal \{\.\.\.props\} \/>/);
  assert.match(scope, /className=\{owner\?\.dialogClassName \?\? "options-dialog"\}/);
  assert.match(scope, /dismissLabel=\{owner\?\.dismissLabel \?\? "閉じる"\} blocked=\{owner\?\.blocked\} restoreReady=\{owner\?\.restoreReady\}/);
  assert.match(scope, /state.getView\(retained.current.id\)/);
  assert.doesNotMatch(modal, /invoke\(|setTimeout|\.showModal\(|\.close\(|\.appendChild\(|open && props.children/);
});

test("scope context remains stable during descriptor publication to avoid replacement rerender loops", () => {
  const scope = source("settings-modal-scope.tsx"), modal = source("scoped-modal.tsx");
  assert.match(scope, /const context = useMemo\(\(\) => \(\{ state, id, body, replacement, dialog \}\), \[state, id, body, replacement\]\)/);
  assert.match(scope, /<Context.Provider value=\{context\}/);
  assert.match(modal, /useMemo\(\(\) => \(\{/);
  assert.match(modal, /latest.current.onDismiss\(\)/);
  assert.match(modal, /scope\?\.state.remove\(id\)/);
});

test("import admission, input preservation and cancellation remain owned by the existing controller", () => {
  const panel = source("audio-import.tsx");
  assert.match(panel, /blocked=\{busy \|\| !!snapshot\?\.busy\}/);
  assert.match(panel, /restoreReady=\{!busy && !snapshot\?\.busy\}/);
  assert.match(panel, /if\(snapshotRef.current\?\.busy\|\|busy\)/);
  assert.match(panel, /canStartImport\(snapshot,title,terms,common\?\.lockVersion\?\?null,recordingActive\)/);
  assert.match(panel, /JSON.stringify\(inputRef.current\) === expected && snapshotRef.current\?\.token === s.token/);
  assert.match(panel, /persistentReplacement=\{vocabularyContent\}/);
  assert.match(panel, /aria-controls=\{scope\?\.id\?\?"audio-import-dialog"\}/);
});

test("portaled import fields keep the same grid spacing without overriding hidden views", () => {
  const css = source("prototype-alignment.css");
  assert.match(css, /\.import-dialog \.settings-modal-slot>\.settings-modal-view:not\(\[hidden\]\) \{ display:grid; gap:16px; min-width:0; \}/);
  assert.match(css, /\.import-dialog \.settings-modal-view>:is\(h2,p,dl,fieldset,\.analysis-actions\) \{ margin-block:0; \}/);
});
