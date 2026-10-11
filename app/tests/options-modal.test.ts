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

test("modal entry points share primary actions while fields keep one stable body portal",()=>{
  const modal=source("options-modal.tsx");
  assert.match(modal,/triggerHost \? createPortal\(trigger,triggerHost\) : trigger/);
  assert.match(modal,/data-portaled=\{!!triggerHost\}/);
  assert.match(modal,/ref=\{triggerRef\}[^\n]*hidden=\{hidden\}/);
  assert.doesNotMatch(modal,/createPortal\(children|createPortal\(<CommonModal/);
  assert.match(modal,/:createPortal\(<div[^\n]*data-options-modal-host style=\{\{display:"contents"\}\}/);
  assert.match(modal,/<\/div>,document.body\)/);
  assert.doesNotMatch(modal,/open && createPortal|hidden && createPortal|data-options-modal-host[^\n]*hidden=/);
  const panel=source("minutes.tsx"), editor=source("minutes-editor.tsx");
  assert.match(panel,/triggerHost=\{settingsActionsHost\}/);
  assert.match(panel,/optionsHost=\{editorOptionsHost\}/);
  assert.match(editor,/triggerHost=\{formatActionsHost\} triggerRef=\{formatTrigger\}/);
  assert.match(editor,/triggerLabel="編集の管理"[^\n]*triggerHost=\{optionsHost\}/);
});

test("minutes comparison replaces settings without introducing a second native dialog",()=>{
  const panel=source("minutes.tsx"), modal=source("options-modal.tsx");
  assert.match(modal,/replacement=\{replacement\}/);
  assert.match(panel,/replacement=\{preview \? \{ key: `\$\{preview.view.id\}:\$\{preview.restoring\}`/);
  assert.match(panel,/content: reviewContent, blocked: disabled, onDismiss: \(\) => setPreview\(null\)/);
  assert.match(panel,/opener.current = button;\s*button.focus\(\{ preventScroll: true \}\)/);
  assert.doesNotMatch(panel,/<CommonModal[^\n]*open=\{!!preview\}/);
});

test("closing detailed formatting resumes the new field without stealing a later focus move",()=>{
  const editor=source("minutes-editor.tsx");
  assert.match(editor,/focusAfterOptions.current=!next;setFormatOpen\(next\)/);
  assert.match(editor,/afterOptions && document.activeElement !== formatTrigger.current && document.activeElement !== document.body/);
  assert.match(editor,/if \(!afterConfirmation && !afterOptions\) \{ focus\(\); return; \}/);
  assert.match(editor,/requestAnimationFrame\(focus\)/);
  assert.match(editor,/busy, locked,formatOpen\]/);
});

test("block deletion retains text selection while explicitly restoring its pointer-invoked toolbar opener",()=>{
  const editor=source("minutes-editor.tsx");
  assert.match(editor,/onMouseDown=\{\(e\) => e.preventDefault\(\)\}/);
  assert.match(editor,/onClick=\{event => action\(event.currentTarget\)\}/);
  assert.match(editor,/toolbarButton\("ブロックを削除", opener =>/);
  assert.match(editor,/void confirm\(\{ opener, title: "議事録ブロックの削除を確認"/);
});

test("transcript merge and bulk confirmations restore the clicked action even when WebKit does not focus buttons",()=>{
  const transcript=source("transcript.tsx");
  for (const title of ["話者の統合を確認", "複数発言の話者変更を確認"]) {
    const request=transcript.indexOf(`title: "${title}"`);
    assert.ok(request>=0,title);
    const handler=transcript.lastIndexOf("onClick=",request);
    const action=transcript.slice(handler,request);
    assert.match(action,/^onClick=\{event=>\{/u,title);
    assert.match(action,/void confirm\(\{ opener: event.currentTarget, $/u,title);
    assert.doesNotMatch(action,/\.focus\(|await /u,"retain the exact action synchronously without moving the selected field");
  }
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
