import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const guide = readFileSync(new URL("../src/model-import-guide.tsx", import.meta.url), "utf8");
const owner = readFileSync(new URL("../src/model-inspection.tsx", import.meta.url), "utf8");
test("native model selection captures and restores focus only inside the still active modal", () => {
  assert.match(guide, /element\.focus\(\{ preventScroll: true \}\)/);
  assert.match(guide, /useLayoutEffect\(\(\) =>/);
  assert.match(guide, /if \(picking \|\| disabled \|\| !focusReturn\.current\) return/);
  assert.match(guide, /requestAnimationFrame\(restore\)/);
  assert.match(guide, /window\.setTimeout\(.*150\)/);
  assert.match(guide, /active !== element && active !== document\.body && active !== modal/);
  assert.match(guide, /mounted\.current && request === sequence\.current && !current\.current\.disabled && element\.isConnected && !element\.disabled && topOpenModal\(\)\?\.contains\(element\)/);
  assert.match(guide, /select\(item\.role, event\.currentTarget\)/);
});
test("guided inspection and copy use the same reviewed declaration with no automatic execute or apply", () => {
  assert.match(owner, /if \(source\.manifest\) assertGuidedInspection\(result, source as GuidedModelSource\)/);
  assert.match(owner, /reviewSource\.current = structuredClone\(source\)/);
  assert.match(owner, /reviewSource\.current === source/);
  assert.match(owner, /"place_model_package", \{ token, \.\.\.source, expectedReviewHash: reviewHash \}/);
  assert.doesNotMatch(guide, /check_model_compatibility|apply_model_profile_application|place_model_package/);
  assert.match(guide, /result === null\) return/);
  assert.match(guide, /current\.current\.engine !== selectedEngine/);
});
