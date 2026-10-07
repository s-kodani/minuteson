import assert from "node:assert/strict";
import test from "node:test";
import { cancelTextEdit, textEditBaseline } from "../src/transcript-edit-session.ts";
import { DraftSaveQueue } from "../src/draft-save.ts";
import { canonicalPayload, type DraftPayload, type Utterance } from "../src/transcript-state.ts";
const payload = (): DraftPayload => ({ schema_version: 1, speaker_names: [], utterances: [] });
const utterance: Utterance = { id: "u", startUs: 0, endUs: 1, effectiveText: "点検日", recognizedText: "点検日", speakerIds: ["a"], attributionState: "assigned" };
test("cancel removes only a new text override, preserving unrelated edits and payload structures", () => {
  const original = payload(), baseline = textEditBaseline(original, utterance);
  const current: DraftPayload = { ...original, speaker_names: [{ speaker_id: "a", display_name: "名前" }], manual_speakers: [{ id: "b", label: "追加" }], utterances: [{ utterance_id: "u", effective_text: "誤り", speaker_ids: ["a"] }, { utterance_id: "v", effective_text: "他の発言", speaker_ids: [] }] };
  const before = structuredClone(current), result = cancelTextEdit(current, baseline);
  assert.deepEqual(current, before); assert.deepEqual(original, payload());
  assert.deepEqual(result, { ...current, utterances: [current.utterances[1]] });
});
test("cancel restores pre-existing text but never undoes a subsequent speaker assignment", () => {
  for (const hadOverride of [false, true]) {
    const original = payload(); if (hadOverride) original.utterances.push({ utterance_id: "u", effective_text: "前の本文", speaker_ids: ["a"] });
    const baseline = textEditBaseline(original, utterance);
    const current = { ...original, utterances: [{ utterance_id: "u", effective_text: "途中", speaker_ids: ["b"] }] };
    assert.deepEqual(cancelTextEdit(current, baseline).utterances, [{ utterance_id: "u", effective_text: hadOverride ? "前の本文" : "点検日", speaker_ids: ["b"] }]);
  }
});
test("a split part's focus baseline is its own effective text, not the full recognized original", () => {
  const part = { ...utterance, id: "split:part", effectiveText: "後半", recognitionIsSource: true, sourceUtteranceId: "u" };
  const p = payload(), baseline = textEditBaseline(p, part);
  assert.equal(baseline.text, "後半");
  assert.deepEqual(cancelTextEdit({ ...p, utterances: [{ utterance_id: part.id, effective_text: "編集", speaker_ids: ["a"] }] }, baseline), p);
});
test("cancel before autosave produces no write; cancel during autosave is saved without erasing history", async () => {
  const saved: DraftPayload[] = []; let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const queue = new DraftSaveQueue(async (id, lockVersion, p: DraftPayload) => { saved.push(structuredClone(p)); await gate; return { id, lockVersion: lockVersion + 1, payload: p }; }, canonicalPayload);
  queue.load({ id: "d", lockVersion: 0, payload: payload() }); const baseline = textEditBaseline(payload(), utterance);
  const edit = () => queue.edit(p => ({ ...p, utterances: [{ utterance_id: "u", effective_text: "途中", speaker_ids: ["a"] }] }));
  edit(); queue.edit(p => cancelTextEdit(p, baseline)); await queue.flush(); assert.equal(saved.length, 0);
  edit(); const saving = queue.flush(); queue.edit(p => cancelTextEdit(p, baseline)); release(); await saving;
  assert.equal(saved.length, 2); assert.equal(saved[0].utterances[0].effective_text, "途中"); assert.deepEqual(saved[1], payload());
  assert.equal(queue.snapshot().draft?.lockVersion, 2); assert.equal(queue.snapshot().dirty, false);
});
test("cancel after acknowledged autosave adds a restoration rather than mutating the acknowledged draft", async () => {
  const saved: DraftPayload[] = [];
  const queue = new DraftSaveQueue(async (id, lockVersion, p: DraftPayload) => { saved.push(structuredClone(p)); return { id, lockVersion: lockVersion + 1, payload: p }; }, canonicalPayload);
  queue.load({ id: "d", lockVersion: 0, payload: payload() }); const baseline = textEditBaseline(payload(), utterance);
  queue.edit(p => ({ ...p, utterances: [{ utterance_id: "u", effective_text: "保存された途中", speaker_ids: ["a"] }] })); await queue.flush();
  queue.edit(p => cancelTextEdit(p, baseline)); assert.equal(queue.snapshot().draft?.payload.utterances[0].effective_text, "保存された途中"); await queue.flush();
  assert.equal(saved.length, 2); assert.equal(queue.snapshot().draft?.lockVersion, 2); assert.deepEqual(saved[1], payload());
});
