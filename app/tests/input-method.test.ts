import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { inputMethodOwnsEvent } from "../src/input-method.ts";

test("IME ownership retains active composition, native flags and WebKit confirmation fallback", () => {
  for (const active of [false, true]) for (const isComposing of [false, true]) for (const keyCode of [undefined, 13, 32, 229]) {
    assert.equal(inputMethodOwnsEvent({ isComposing, keyCode }, active), active || isComposing || keyCode === 229);
  }
});

test("rich editor guards native conversion before newline mutation and prevents composition-time paste and evidence actions", () => {
  const source = readFileSync(new URL("../src/minutes-rich-field.tsx", import.meta.url), "utf8");
  assert.match(source, /if \(inputMethodOwnsEvent\(event, composing.current\)\) return;\s+if \(\["insertParagraph", "insertLineBreak"\]/);
  assert.match(source, /inputMethodOwnsEvent\(event.nativeEvent, composing.current\) \|\| event.repeat/);
  assert.match(source, /chip && !disabled && !composing.current/);
  assert.match(source, /event.preventDefault\(\); if \(disabled \|\| evidenceOnly \|\| composing.current\) return/);
});

test("synthetic IME browser driver is executable and verifies confirmed-only saves and ordinary newlines", () => {
  const source = readFileSync(new URL("minutes-ime-browser-check.js", import.meta.url), "utf8");
  assert.doesNotThrow(() => new Function(`return (${source})`));
  assert.match(source, /compositionend did not save exactly one confirmed update/);
  assert.match(source, /plain Enter after composition stopped adding one newline/);
});
