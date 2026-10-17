import test from "node:test";
import assert from "node:assert/strict";
import { DraftSaveQueue } from "../src/draft-save.ts";
import { promptValidation, resetPrompt, type MinutesPromptSnapshot } from "../src/minutes-prompt-state.ts";
const defaults = { schemaVersion: 1 as const, extractPrompt: "既定の作成", mergePrompt: "既定の統合" };

test("prompts accept arbitrary instructions, measure UTF-8 bytes, and reset one field", () => {
  assert.equal(promptValidation({ ...defaults, extractPrompt: "表形式で自由に出力" }), "");
  for (const extractPrompt of [" ", "a\0b", "あ".repeat(5462)]) assert.ok(promptValidation({ ...defaults, extractPrompt }));
  assert.equal(promptValidation({ ...defaults, extractPrompt: "a".repeat(16384) }), "");
  assert.deepEqual(resetPrompt({ ...defaults, extractPrompt: "変更", mergePrompt: "変更した統合" }, defaults, "extractPrompt"), { ...defaults, mergePrompt: "変更した統合" });
});

test("lost prompt save responses recover without discarding later edits or overwriting conflicts", async () => {
  let saved: MinutesPromptSnapshot = { id: "prompts", lockVersion: 0, payload: defaults, defaults };
  let lose = true;
  const queue = new DraftSaveQueue(async (_id, version, payload: typeof defaults) => {
    assert.equal(version, saved.lockVersion);
    saved = { ...saved, lockVersion: version + 1, payload };
    if (lose) { lose = false; throw Error("lost response"); }
    return saved;
  }, payload => ({ ...payload }), async () => saved);
  queue.load(saved);
  queue.edit(value => ({ ...value, extractPrompt: "first" }));
  await assert.rejects(queue.flush());
  queue.edit(value => ({ ...value, extractPrompt: "later" }));
  await queue.retry();
  assert.equal(saved.payload.extractPrompt, "later");
  assert.equal(queue.snapshot().dirty, false);
  lose = true;
  queue.edit(value => ({ ...value, extractPrompt: "my input" }));
  await assert.rejects(queue.flush());
  saved = { ...saved, lockVersion: saved.lockVersion + 1, payload: { ...defaults, extractPrompt: "other window" } };
  await assert.rejects(queue.retry(), /別の内容/);
  assert.equal(queue.snapshot().payload?.extractPrompt, "my input");
});
