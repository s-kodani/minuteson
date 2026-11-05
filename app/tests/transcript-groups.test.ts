import assert from "node:assert/strict";
import test from "node:test";
import { groupTranscriptRows } from "../src/transcript-groups.ts";
import { retainedTextRows } from "../src/transcript-edit-session.ts";
import type { Utterance } from "../src/transcript-state.ts";

const row = (id: string, startUs: number, patch: Partial<Utterance> = {}): Utterance => ({
  id, trackId: "mic", startUs, endUs: startUs + 1_000_000, speakerIds: ["a"],
  attributionState: "assigned", effectiveText: "発言。", recognizedText: "発言。", ...patch,
});
const ids = (groups: Utterance[][]) => groups.map(group => group.map(row => row.id));

test("consecutive speech is grouped without changing text, identity or playback bounds", () => {
  const rows = [row("a", 0), row("b", 1_000_000), row("c", 4_000_000)], before = structuredClone(rows);
  const groups = groupTranscriptRows(rows, rows);
  assert.deepEqual(ids(groups), [["a", "b", "c"]]);
  assert.equal(groups[0][1], rows[1]);
  assert.deepEqual(rows, before);
});

test("speaker changes, track changes, pauses and temporal overlaps keep separate paragraphs", () => {
  for (const patch of [
    { speakerIds: ["b"] }, { trackId: "system" }, { trackId: undefined },
    { startUs: 3_000_001 }, { startUs: 999_999 },
    { speakerIds: [], attributionState: "unknown" },
    { speakerIds: ["a", "b"], attributionState: "overlap" },
    { attributionState: "needs_split" }, { sourceUtteranceId: "original" },
  ]) {
    const rows = [row("a", 0), row("b", 1_000_000, patch)];
    assert.deepEqual(ids(groupTranscriptRows(rows, rows)), [["a"], ["b"]], JSON.stringify(patch));
    assert.deepEqual(ids(groupTranscriptRows(rows, [...rows].reverse())), [["b"], ["a"]]);
  }
});

test("search and retained edits never join over a hidden intervening turn", () => {
  const rows = [row("a", 0), row("b", 1_000_000, { speakerIds: ["b"] }), row("c", 2_000_000)];
  assert.deepEqual(ids(groupTranscriptRows(rows, [rows[0], rows[2]])), [["a"], ["c"]]);
  const shown = retainedTextRows(rows, [rows[2]], ["a"]).rows;
  assert.deepEqual(ids(groupTranscriptRows(rows, shown)), [["a"], ["c"]]);
  assert.deepEqual(groupTranscriptRows(rows, []), []);
});

test("continuous monologues get a paragraph break after a minute", () => {
  const rows = Array.from({ length: 61 }, (_, i) => row(String(i), i * 1_000_000));
  const groups = groupTranscriptRows(rows, rows);
  assert.deepEqual(groups.map(group => group.length), [60, 1]);
});

test('automatic identification scores remain on their original rows instead of being consolidated away',()=>{
  const rows=[row('a',0),row('b',1_000_000,{speakerIdentification:{similarityMilli:530,runnerUpMilli:200,needsReview:true,reason:'nearest_below_threshold'}}),row('c',2_000_000)];
  assert.deepEqual(ids(groupTranscriptRows(rows,rows)),[['a'],['b'],['c']]);
});
