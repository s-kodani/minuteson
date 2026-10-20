import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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
