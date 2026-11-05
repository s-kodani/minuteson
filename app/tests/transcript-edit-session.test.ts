import assert from "node:assert/strict";
import test from "node:test";
import { retainedTextRows } from "../src/transcript-edit-session.ts";
import { canonicalPayload, type DraftPayload, type Utterance } from "../src/transcript-state.ts";
const utterance: Utterance = { id: "u", startUs: 0, endUs: 1, effectiveText: "点検日", recognizedText: "点検日", speakerIds: ["a"], attributionState: "assigned" };
test("all matching rows and retained edits stay in canonical order without pagination", () => {
  const rows = Array.from({ length: 103 }, (_, i) => ({ ...utterance, id: String(i) })), before = structuredClone(rows);
  const result = retainedTextRows(rows, rows, ["0", "102", "0", "missing"]);
  assert.deepEqual(result.rows.map(item => item.id), rows.map(item => item.id)); assert.equal(result.extra, 0);
  assert.deepEqual(retainedTextRows(rows, [], ["102", "0"]).rows.map(item => item.id), ["0", "102"]);
  assert.deepEqual(retainedTextRows(rows, [], ["missing"]), { rows: [], extra: 0 }); assert.deepEqual(rows, before);
});
