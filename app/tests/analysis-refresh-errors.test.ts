import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

test("analysis polling resolves read failures without hiding action or native flow errors",()=>{
  const panel=readFileSync(new URL("../src/analysis.tsx",import.meta.url),"utf8");
  assert.match(panel,/\[refreshError, setRefreshError\] = useState\(""\)/);
  assert.match(panel,/setRefreshError\(""\);\s*\} catch \(cause\) \{ if \(!disposed\) setRefreshError\(String\(cause\)\); \}/);
  assert.match(panel,/await refreshRef\.current\(\); \}\s*catch \(cause\) \{ setError\(String\(cause\)\); \}/);
  assert.match(panel,/message=\{error \|\| refreshError \|\| flow\?\.errorMessage\}/);
});
test("background publication verifies recovery and does not clear a newer action error", () => {
  const source = readFileSync(new URL("../src/analysis-publication.ts", import.meta.url), "utf8");
  assert.match(source, /current\?\.meetingId === meetingId/);
  assert.match(source, /current.id !== failure.flowId/);
  assert.match(source, /current.stage === "review" && !current.errorMessage/);
  assert.match(source, /resolveMatching\(toastKey\(meetingId, "analysis-operation"\), failure.message\)/);
  assert.doesNotMatch(source, /toasts\.resolve\(key\)/);
});
