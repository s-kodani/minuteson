import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/minutes.tsx", import.meta.url), "utf8");
test("plain minutes export keeps its activated icon as the focus owner in WebKit", () => {
  const begin = source.slice(source.indexOf("const exportMarkdown ="), source.indexOf("const jobAction ="));
  assert.match(begin, /exportOpener\.current = opener;[\s\S]*opener\.focus\(\{ preventScroll: true \}\)/);
  assert.ok(begin.indexOf("opener.focus") < begin.indexOf("await withConfirmedEdits"));
  // Completion does not steal another field or focus a disabled/hidden icon.
  assert.match(source, /!target\.disabled && !target\.closest\("\[hidden\],\[inert\]"\)/);
  assert.match(source, /active === document\.body \|\| active === target \|\| !active\?\.isConnected/);
});
