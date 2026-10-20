import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const view = readFileSync(new URL("../src/vocabulary.tsx", import.meta.url), "utf8");
const analysis = readFileSync(new URL("../src/analysis.tsx", import.meta.url), "utf8");


test("vocabulary presentation preserves persistence and explicit destructive decisions without duplicating notification effects", () => {
  assert.match(view, /registerPendingEdits\(async \(\) => \{\s*await queue.flush\(\)/);
  assert.match(view, /if \(loading \|\| confirm.pending \|\| !state.dirty/);
  assert.match(view, /validate: \(\) => JSON.stringify\(queue.snapshot\(\).payload\) === expected/);
  assert.match(view, /action: async \(\) => \{ queue.edit\(value => value.filter/);
  assert.match(view, /if \(validVocabulary\(queue.snapshot\(\).payload \?\? \[\]\)\) await queue.flush\(\)/);
  const persistent = view.slice(view.indexOf("const persistentEditor"), view.indexOf("const view: ModalReplacement"));
  assert.doesNotMatch(persistent, /<IssueStatus/);
  assert.doesNotMatch(persistent, /\{issue\}|<IssueNotice/);
  assert.equal((view.match(/new DraftSaveQueue/g) ?? []).length, 1);
});
