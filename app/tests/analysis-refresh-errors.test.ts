import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

test("analysis polling resolves read failures without hiding action or native flow errors",()=>{
  const panel=readFileSync(new URL("../src/analysis.tsx",import.meta.url),"utf8");
  assert.match(panel,/\[refreshError, setRefreshError\] = useState\(""\)/);
  assert.match(panel,/observeInvalidations\(\{/);
  assert.match(panel,/setRefreshError\(""\);/);
  assert.match(panel,/error: cause => \{ if \(!disposed\) setRefreshError\(String\(cause\)\); \}/);
  assert.match(panel,/await refreshRef\.current\(\); \}\s*catch \(cause\) \{ if \(!\(cause instanceof SavedEditsCancelled\)\) setError\(String\(cause\)\); \}/);
  assert.match(panel,/message=\{error \|\| flow\?\.errorMessage\}/);
  assert.match(panel,/historyOnly=\{!error && flowHistoryOnly\}/);
  assert.match(panel,/refreshError && <p role="status">/);
  assert.match(panel,/if \(previousFlow === undefined\) setFlowHistoryOnly\(true\)/);
  assert.match(panel,/else if \(previousFlow !== receipt\) setFlowHistoryOnly\(false\)/);
});
test("background publication verifies recovery and does not clear a newer action error", () => {
  const source = readFileSync(new URL("../src/analysis-publication.ts", import.meta.url), "utf8");
  assert.match(source, /current\?\.meetingId === meetingId/);
  assert.match(source, /current.id !== failure.flowId/);
  assert.match(source, /current.stage === "review" && !current.errorMessage/);
  assert.match(source, /resolveMatching\(toastKey\(meetingId, "analysis-operation"\), failure.message\)/);
  assert.doesNotMatch(source, /toasts\.resolve\(key\)/);
});
