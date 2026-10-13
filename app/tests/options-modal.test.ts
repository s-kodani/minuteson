import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");

test("modal settings keep inputs mounted and only explicit opening changes presentation",()=>{
  const view=source("options-modal.tsx");
  assert.match(view,/onClick=\{event=>\{event.currentTarget.focus\(\{preventScroll:true\}\);change\(true\);entry.current=event.currentTarget;\}\}/);
  assert.match(view,/open=\{open && !hidden\}/);
  assert.match(view,/onDismiss=\{\(\)=>change\(false\)\}/);
  assert.match(view,/\{children\}<\/div>/);
  assert.doesNotMatch(view,/useEffect|invoke\(|\.flush\(|open && children/);
});

test("macOS pointer opening captures the settings entry point before modal focus moves",()=>{
  for (const [name, open] of [["options-modal.tsx", "change(true)"]]) {
    const view=source(name);
    const focus=view.indexOf("event.currentTarget.focus({preventScroll:true});");
    assert.ok(focus>=0,name);
    assert.equal(view.slice(focus+"event.currentTarget.focus({preventScroll:true});".length).startsWith(open+";"),true,name);
  }
  assert.match(source("meeting-audio.tsx"),/<OptionsModal label="再生設定" icon="more"/);
});

test("modal entry points share primary actions while fields keep one stable body portal", () => {
  const modal=source("options-modal.tsx"), panel=source("minutes.tsx"), editor=source("minutes-markdown-editor.tsx");
  assert.match(modal,/triggerHost \? createPortal\(trigger,triggerHost\) : trigger/);
  assert.match(modal,/data-portaled=\{!!triggerHost\}/);
  assert.doesNotMatch(modal,/open && createPortal|hidden && createPortal/);
  assert.match(panel,/actionsHost=\{actionsHost\}/);
  assert.match(editor,/createPortal\(actions, actionsHost\)/);
});

test("plain minutes use direct controls without an operations dialog", () => {
  const panel=source("minutes.tsx"), modal=source("options-modal.tsx");
  assert.match(modal,/replacement=\{replacement\}/);
  assert.doesNotMatch(panel,/OptionsModal|議事録の操作|retry_minutes_generation/);
  assert.match(panel,/>議事録を生成<\/button>/);
  assert.doesNotMatch(panel,/replacement=|MinutesDocument|setPreview|直前版|候補を採用/);
});

test("meeting management is available only in detail, never as a list footer", () => {
  const view=source("main.tsx");
  assert.match(view,/const renderMeetingTools = .*=> !detail \? null : <OptionsModal/);
  assert.doesNotMatch(view,/renderMeetingTools\(null, null\)|landing-tools/);
  assert.match(view,/<RowMenu label=\{`\$\{meeting.title\}の操作`\}/);
});




test("nested settings restore to their parent but unrelated dialogs remain queued",()=>{
  const modal=source("common-modal.tsx");
  assert.match(modal,/parent && \(nested \|\| parent.hasAttribute\("data-settings-modal"\)\)/);
  assert.match(modal,/top && top !== parentModal.current/);
  assert.match(modal,/remaining && remaining !== parentModal.current/);
  assert.match(modal,/event.target.closest\("dialog\[open\]"\) !== element/);
  assert.match(modal,/latest.current.restoreReady/);
  assert.match(modal,/if \(!latest.current.blocked\)/);
});

test("settings use one outer scrollport without scrolling individual toolbars or notices",()=>{
  const css=source("prototype-alignment.css");
  assert.match(css,/\.options-dialog \{[^}]*max-height:calc\(100dvh - 48px\);[^}]*overflow:auto;/);
  assert.match(css,/\.options-dialog \.workspace-tools-body \{ max-height:none; overflow:visible;/);
  assert.match(css,/\.options-dialog \.issue-status \{ max-height:none; overflow:visible;/);
});
