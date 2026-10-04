import test from "node:test";
import assert from "node:assert/strict";
import { effectiveVocabulary, validVocabulary, type VocabularyEntry } from "../src/vocabulary-state.ts";
import { DraftSaveQueue, registerPendingEdits, flushPendingEdits } from "../src/draft-save.ts";

const entry = (term: string, enabled = true): VocabularyEntry => ({ id: crypto.randomUUID(), term, reading: null, note: null, enabled });
test("incomplete or duplicate rows wait for input rather than trigger autosave errors", () => {
  assert.ok(validVocabulary([])); assert.ok(validVocabulary([{ ...entry("点検日"), reading: "てんけんび", note: "補足\n次の行\t注記" }]));
  for (const items of [[entry("")], [entry(" 点検日")], [entry("MON"), entry("MON")], [entry("x\0")], [entry("x".repeat(201))], [{ ...entry("点検日"), reading: " " }], [{ ...entry("点検日"), note: "\r" }]]) assert.equal(validVocabulary(items), false);
});
test("enabled meeting terms override exact common spelling without rewriting inputs", () => {
  const common = [entry("点検日"), entry("MON"), entry("無効", false)];
  const meeting = [{ ...entry("点検日"), reading: "てんけんび", note: "会議別" }, entry("MON", false), entry("mon")];
  const before = structuredClone({ common, meeting });
  assert.deepEqual(effectiveVocabulary(common, meeting), [meeting[0], meeting[2], common[1]]);
  assert.deepEqual({ common, meeting }, before);
  assert.deepEqual(effectiveVocabulary([], []), []);
});
test("vocabulary saves preserve edits made while a write is in flight", async () => {
  let version = 0;
  const saves: VocabularyEntry[][] = [];
  let release!: () => void;
  const wait = new Promise<void>((resolve) => { release = resolve; });
  const queue = new DraftSaveQueue(async (id: string, expected: number, payload: VocabularyEntry[]) => {
    assert.equal(expected, version); saves.push(structuredClone(payload));
    if (saves.length === 1) await wait;
    return { id, payload: structuredClone(payload), lockVersion: ++version };
  }, (payload: VocabularyEntry[]) => structuredClone(payload));
  queue.load({ id: "global", payload: [], lockVersion: 0 });
  const first = entry("点検日"); queue.edit(() => [first]);
  const saving = queue.flush(); queue.edit((items) => [{ ...items[0], reading: "てんけんび" }]);
  release(); await saving;
  assert.equal(saves.length, 2); assert.equal(queue.snapshot().draft?.lockVersion, 2);
  assert.equal(queue.snapshot().dirty, false); assert.equal(queue.snapshot().payload?.[0].reading, "てんけんび");
});
test("vocabulary save failure blocks navigation and keeps input until explicit retry", async () => {
  let fail = true;
  const queue = new DraftSaveQueue(async (id: string, _expected: number, payload: VocabularyEntry[]) => {
    if (fail) throw new Error("保存競合");
    return { id, payload, lockVersion: 1 };
  }, (payload: VocabularyEntry[]) => structuredClone(payload));
  queue.load({ id: "global", payload: [], lockVersion: 0 }); queue.edit(() => [entry("点検日")]);
  const unregister = registerPendingEdits(() => queue.flush());
  try {
    await assert.rejects(flushPendingEdits(), /保存競合/);
    assert.equal(queue.snapshot().payload?.[0].term, "点検日");
    fail = false; await assert.rejects(flushPendingEdits(), /保存競合/);
    await queue.retry(); await flushPendingEdits(); assert.equal(queue.snapshot().dirty, false);
  } finally { unregister(); }
});
