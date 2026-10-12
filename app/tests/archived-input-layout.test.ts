import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

test("archived input history grows without its own capped scrollport", () => {
  const base = readFileSync(new URL("../src/style.css", import.meta.url), "utf8");
  const layout = readFileSync(new URL("../src/prototype-alignment.css", import.meta.url), "utf8");
  assert.match(base, /\.archived-input-notices \{ flex:none; min-width:0; max-height:none; overflow:visible; \}/);
  assert.doesNotMatch(base, /\.meeting-detail-content \.archived-input-notices[^}]*max-height:16dvh/);
  assert.match(layout, /\.content\.meeting-detail-content:has\(>\.archived-input-notices:not\(\[hidden\]\)\) \{ overflow-y:auto; \}/);
  assert.match(layout, /\.meeting-detail-content:has\(>\.archived-input-notices details\[open\]\)>\.workspace-tab-panel \{ flex:1 0 min\(480px,60dvh\); \}/);
});
