import assert from "node:assert/strict";
import test from "node:test";
import { appShortcut } from "../src/keyboard.ts";
const key = (value: string, overrides = {}) => ({ key: value, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, repeat: false, isComposing: false, defaultPrevented: false, ...overrides });
test("both app command modifiers map N/F/Enter/Z; unmodified Space and Esc only", () => {
  for (const modifier of [{ ctrlKey: true }, { metaKey: true }]) {
    for (const [input, action] of [["n", "recording"], ["N", "recording"], ["f", "search"], ["Enter", "commit"], ["z", "undo"]]) assert.equal(appShortcut(key(input, modifier)), action);
    assert.equal(appShortcut(key(" ", modifier)), null);
    assert.equal(appShortcut(key("Escape", modifier)), null);
  }
  assert.equal(appShortcut(key(" ")), "playback"); assert.equal(appShortcut(key("Escape")), "cancel");
  for (const input of ["n", "f", "z", "Enter", "Tab", "F1"]) assert.equal(appShortcut(key(input)), null);
});
test("IME, held keys, consumed events, alt/shift and both command modifiers never dispatch", () => {
  for (const overrides of [{ repeat: true }, { isComposing: true }, { keyCode: 229 }, { defaultPrevented: true }, { altKey: true }, { shiftKey: true }, { ctrlKey: true, metaKey: true }]) {
    for (const input of ["n", "f", "Enter", "z", " ", "Escape"]) assert.equal(appShortcut(key(input, { ctrlKey: input !== " " && input !== "Escape", ...overrides })), null);
  }
});
