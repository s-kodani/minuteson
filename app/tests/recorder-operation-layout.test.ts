import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

test("retained recorder notices share header gutters without padding an empty host", () => {
  const source = readFileSync(new URL("../src/recorder-operation-notices.tsx", import.meta.url), "utf8");
  const css = readFileSync(new URL("../src/prototype-alignment.css", import.meta.url), "utf8");
  assert.match(source, /className="recorder-operation-notifications" aria-label="録音パネルからの操作通知"/);
  assert.match(css, /\.recorder-operation-notifications \{ flex:none; min-width:0; --recorder-notice-gutter:28px; \}/);
  assert.match(css, /\.recorder-operation-notifications>\.issue-status \{ margin:0; padding:12px var\(--recorder-notice-gutter\); \}/);
  assert.match(css, /\.recorder-operation-notifications>\.issue-status>strong \{ min-width:0; overflow-wrap:anywhere; \}/);
  assert.match(css, /\.main-column:has\(\.meeting-detail-status\)>\.recorder-operation-notifications \{ --recorder-notice-gutter:16px; \}/);
  assert.doesNotMatch(css, /\.recorder-operation-notifications \{[^}]*padding:/);
});
