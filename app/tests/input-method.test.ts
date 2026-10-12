import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { inputMethodOwnsEvent } from "../src/input-method.ts";

test("IME ownership retains active composition, native flags and WebKit confirmation fallback", () => {
  for (const active of [false, true]) for (const isComposing of [false, true]) for (const keyCode of [undefined, 13, 32, 229]) {
    assert.equal(inputMethodOwnsEvent({ isComposing, keyCode }, active), active || isComposing || keyCode === 229);
  }
});



test("plain Markdown editing keeps native input history and guards explicit saving during IME", () => {
  const source = readFileSync(new URL("../src/minutes-markdown-editor.tsx", import.meta.url), "utf8");
  assert.match(source, /<textarea/);
  assert.doesNotMatch(source, /contentEditable|execCommand|onBeforeInput|onPaste/);
  assert.match(source, /onCompositionStart=\{\(\)=>controller.setComposing\(true\)\}/);
  assert.match(source, /onCompositionEnd=\{\(\)=>controller.setComposing\(false\)\}/);
  assert.match(source, /if\(!event.nativeEvent.isComposing\)void save\(\)/);
  assert.match(source, /onChange=\{event=>controller.edit\(event.target.value\)\}/);
  assert.doesNotMatch(source, /setInterval\(|setTimeout\(/);
});
