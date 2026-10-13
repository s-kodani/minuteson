import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { visibleModalTarget } from "../src/modal-state.ts";

const target = ({disabled=false,hidden=false,inert=false,rects=1,connected=true}={}) => ({
  isConnected:connected,
  matches:(selector:string)=>selector===":disabled"&&disabled,
  closest:(selector:string)=>selector==="[hidden], [inert]"&&(hidden||inert)?{}:null,
  getClientRects:()=>Array.from({length:rects},()=>({})),
}) as unknown as HTMLElement;
const root = (...targets:HTMLElement[]) => ({querySelectorAll:()=>targets}) as unknown as ParentNode;

test("disabled header cannot select a hidden sibling's enabled cancel",()=>{
  const header=target({disabled:true}),hidden=target({hidden:true,rects:0});
  assert.equal(visibleModalTarget(root(header,hidden),"[data-dialog-cancel]"),null);
  const ready=target();
  assert.equal(visibleModalTarget(root(ready,hidden),"[data-dialog-cancel]"),ready);
});
test("visible focus selection rejects hidden ancestors, inert views, detached and unrendered nodes",()=>{
  const visible=target();
  assert.equal(visibleModalTarget(root(target({hidden:true}),target({inert:true}),target({connected:false}),target({rects:0}),visible),"h2, h3"),visible);
});
test("visible focus selection keeps displayed order and never fabricates an unavailable action",()=>{
  const first=target(),second=target();
  assert.equal(visibleModalTarget(root(first,second),"[data-dialog-cancel]"),first);
  assert.equal(visibleModalTarget(root(),"[data-dialog-cancel]"),null);
});
test("initial busy focus remains pending until a visible cancellation is ready without stealing fields or confirmation ownership",()=>{
  const view=readFileSync(new URL("../src/common-modal.tsx",import.meta.url),"utf8");
  assert.match(view,/const cancel = visibleModalTarget\(element, "\[data-dialog-cancel\]"\);\s*const heading = visibleModalTarget\(element, "h2, h3"\);\s*initialFocusPending.current = !cancel/);
  assert.match(view,/blocked \|\| owned \|\| replacement \|\| !initialFocusPending.current/);
  assert.match(view,/active === element \|\| active === document.body \|\| active === visibleModalTarget\(element, "h2, h3"\)/);
  assert.match(view,/if \(!cancel\) return;\s*cancel.focus\(\{ preventScroll: true \}\)/);
  assert.match(view,/\[open, blocked, owned\?\.id, replacement\?\.key, ref\]/);
});
test("plain minutes leave remembers its persistent input before native quit locks lose focus and respects nested reviews",()=>{
  const panel=readFileSync(new URL("../src/minutes.tsx",import.meta.url),"utf8");
  const editor=readFileSync(new URL("../src/minutes-markdown-editor.tsx",import.meta.url),"utf8");
  assert.match(panel,/opener: !topOpenModal\(\) && input\?\.isConnected\s*&& !!input\.getClientRects\(\)\.length \? input : undefined/);
  assert.match(panel,/confirmMarkdownLeave\(controller, live\.current\.meetingTitle, read, confirmLeave\)/);
  assert.match(panel,/inputRef=\{editorInput\}/);
  assert.match(editor,/const textarea=inputRef\?\?ownInput/);
  assert.match(editor,/<textarea ref=\{textarea\}/);
});
