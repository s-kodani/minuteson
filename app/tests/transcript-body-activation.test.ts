import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { shouldOpenTranscriptBody, shouldOpenTranscriptBodyFromKey } from "../src/transcript-body-activation.ts";

const inside = {} as Node, outside = {} as Node;
const contains = (node: Node) => node === inside;
const selected = (anchorNode: Node | null, focusNode: Node | null) => ({ isCollapsed: false, rangeCount: 1, anchorNode, focusNode });

test("a pointer drag within the body keeps the text selection rather than entering editing", () => {
  assert.equal(shouldOpenTranscriptBody(1, selected(inside, inside), contains), false);
  assert.equal(shouldOpenTranscriptBody(2, selected(inside, inside), contains), false);
});
test("selection crossing either body boundary also stays copyable", () => {
  assert.equal(shouldOpenTranscriptBody(1, selected(inside, outside), contains), false);
  assert.equal(shouldOpenTranscriptBody(1, selected(outside, inside), contains), false);
});
test("an ordinary click with no range or just a caret still opens the editor", () => {
  assert.equal(shouldOpenTranscriptBody(1, null, contains), true);
  assert.equal(shouldOpenTranscriptBody(1, { ...selected(inside, inside), isCollapsed: true }, contains), true);
  assert.equal(shouldOpenTranscriptBody(1, { ...selected(inside, inside), rangeCount: 0 }, contains), true);
});
test("a selection elsewhere cannot disable the body's edit action", () => {
  assert.equal(shouldOpenTranscriptBody(1, selected(outside, outside), contains), true);
  assert.equal(shouldOpenTranscriptBody(1, selected(null, null), contains), true);
});
test("keyboard and assistive activation can explicitly edit a selected body", () => {
  assert.equal(shouldOpenTranscriptBody(0, selected(inside, inside), contains), true);
});

const bodyKey = (key: string, overrides = {}) => ({ key,repeat:false,isComposing:false,keyCode:0,defaultPrevented:false,altKey:false,ctrlKey:false,metaKey:false,shiftKey:false,...overrides });
test("body keyboard activation accepts only plain Enter or Space", () => {
  for (const key of ["Enter"," "]) assert.equal(shouldOpenTranscriptBodyFromKey(bodyKey(key)),true);
  for (const key of ["Escape","Tab","a","Space"]) assert.equal(shouldOpenTranscriptBodyFromKey(bodyKey(key)),false);
});
test("body activation ignores both IME signals and held or already-handled keys", () => {
  for (const key of ["Enter"," "]) for (const signal of [{isComposing:true},{keyCode:229},{repeat:true},{defaultPrevented:true}]) {
    assert.equal(shouldOpenTranscriptBodyFromKey(bodyKey(key,signal)),false);
  }
});
test("modified Enter and Space do not replace selection with a body editor", () => {
  for (const key of ["Enter"," "]) for (const modifier of ["altKey","ctrlKey","metaKey","shiftKey"]) {
    assert.equal(shouldOpenTranscriptBodyFromKey(bodyKey(key,{[modifier]:true})),false);
  }
});

test("keyboard browser driver uses visible controls for commit, undo, cancel and modal return", () => {
  const driver=readFileSync(new URL("./keyboard-primary-browser-check.js",import.meta.url),"utf8");
  const runner=readFileSync(new URL("./run-browser-checks.mjs",import.meta.url),"utf8");
  assert.match(runner,/\["keyboard-primary", "browser-fixture.js", "keyboard-primary-browser-check.js"\]/);
  for (const key of ["Meta+f","Enter","Space","Meta+Enter","Meta+z","Escape","Shift+Tab"]) assert.ok(driver.includes(`"${key}"`));
  assert.doesNotMatch(driver,/window\.|monBrowserFixture|dispatchEvent/);
  assert.doesNotThrow(()=>new Function(`return (${driver})`));
});
