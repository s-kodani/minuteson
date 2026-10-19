import test from "node:test";
import assert from "node:assert/strict";
import { cancelName, nameBaseline, namesDirty, stageName } from "../src/speaker-name-edit.ts";
import type { DraftPayload } from "../src/transcript-state.ts";
const payload: DraftPayload = { schema_version:4,...{manual_speakers:[],speaker_merges:[],utterance_splits:[]},speaker_names: [],utterances: [{ utterance_id: "u",effective_text: "本文",speaker_ids: ["a"] }] };
const speaker = { id: "a",label: "話者A",displayName: "田中" };
test("typing keeps names separate; confirming one name preserves every other field", () => {
  const first = { ...nameBaseline(payload,speaker),value: "鈴木" };
  const second = { ...nameBaseline(payload,{ ...speaker,id: "b" }),value: "佐々木" };
  assert.equal(payload.speaker_names.length,0);
  assert.equal(namesDirty({ a: first,b: second },[speaker,{ ...speaker,id: "b" }]),true);
  const saved = stageName(payload,first);
  assert.deepEqual(saved.speaker_names,[{ speaker_id: "a",display_name: "鈴木" }]);
  assert.deepEqual(saved.utterances,payload.utterances);
  assert.equal(namesDirty({ a: first },[{ ...speaker,displayName: "鈴木" }]),false);
  assert.equal(second.value,"佐々木");
});
test("unchanged baseline is a no-op, including empty names", () => {
  assert.equal(stageName(payload,nameBaseline(payload,speaker)),payload);
  assert.equal(stageName(payload,nameBaseline(payload,{ ...speaker,displayName: null })),payload);
});
test("clearing a name explicitly stores null rather than reviving its base name", () => {
  const session = { ...nameBaseline(payload,speaker),value: "" };
  assert.deepEqual(stageName(payload,session).speaker_names,[{ speaker_id: "a",display_name: null }]);
});
test("cancel after a barrier removes only the new override and keeps other names and assignments", () => {
  const baseline = nameBaseline(payload,speaker);
  const current = { ...stageName(payload,{ ...baseline,value: "鈴木" }),speaker_names: [{ speaker_id: "a",display_name: "鈴木" },{ speaker_id: "b",display_name: "佐藤" }] };
  const restored = cancelName(current,baseline);
  assert.deepEqual(restored.speaker_names,[{ speaker_id: "b",display_name: "佐藤" }]);
  assert.deepEqual(restored.utterances,payload.utterances);
});
test("cancel restores a prior saved name override including null without erasing history", () => {
  for (const name of ["山田",null]) {
    const current = { ...payload,speaker_names: [{ speaker_id: "a",display_name: name }] };
    const baseline = nameBaseline(current,{ ...speaker,displayName: name });
    assert.deepEqual(cancelName(stageName(current,{ ...baseline,value: "鈴木" }),baseline),current);
  }
  assert.equal(cancelName(payload,nameBaseline(payload,speaker)),payload);
});
test("name limits count Unicode code points and retain the original input on failure", () => {
  const session = { ...nameBaseline(payload,speaker),value: "😀".repeat(200) };
  assert.equal(stageName(payload,session).speaker_names[0].display_name,session.value);
  session.value += "😀";
  assert.throws(() => stageName(payload,session),/200/);
  assert.equal(Array.from(session.value).length,201);
  assert.equal(payload.speaker_names.length,0);
});
