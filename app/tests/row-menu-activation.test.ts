import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/row-menu.tsx", import.meta.url), "utf8");
test("row menu keeps enabled left-click targets focused before WebKit's default mouse-down", () => {
  const mouseDown = source.slice(source.indexOf("onMouseDownCapture={"), source.indexOf("}} onClick={"));
  assert.match(mouseDown, /event\.button !== 0 \|\| !button \|\| button\.disabled \|\| !event\.currentTarget\.contains\(button\)/);
  assert.match(mouseDown, /event\.preventDefault\(\); button\.focus\(\{ preventScroll: true \}\)/);
  assert.doesNotMatch(mouseDown, /close\(|\.click\(|invoke\(/);
});
test("row menu preserves outside dismissal, keyboard activation and stable confirmation return", () => {
  assert.match(source, /document\.addEventListener\("focusin", outside\)/);
  assert.match(source, /event\.key === "Escape"/);
  assert.match(source, /onClick=\{event =>/);
  assert.match(source, /if \(button && !button\.disabled\) close\(\)/);
  assert.match(source, /export function rowMenuReturnTarget/);
});
