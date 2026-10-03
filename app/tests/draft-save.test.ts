import { test } from "node:test";
import assert from "node:assert/strict";
import { DraftSaveQueue, flushPendingEdits, registerPendingEdits } from "../src/draft-save.ts";

type Payload = { text: string };
type Draft = { id: string; payload: Payload; lockVersion: number };

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test("navigation flush waits for edits made during a save and uses the new version", async () => {
  const calls: { version: number; payload: Payload; response: ReturnType<typeof deferred<Draft>> }[] = [];
  const queue = new DraftSaveQueue<Payload, Draft>((id, version, payload) => {
    const response = deferred<Draft>();
    calls.push({ version, payload, response });
    return response.promise;
  }, (payload) => ({ ...payload }));
  queue.load({ id: "draft", payload: { text: "original" }, lockVersion: 0 });
  queue.edit(() => ({ text: "first" }));
  const unregister = registerPendingEdits(() => queue.flush());
  const saving = queue.flush();
  queue.edit(() => ({ text: "latest" }));
  let navigationDone = false;
  const navigation = flushPendingEdits().then(() => { navigationDone = true; });
  calls[0].response.resolve({ id: "draft", payload: calls[0].payload, lockVersion: 1 });
  await Promise.resolve();
  assert.equal(navigationDone, false);
  assert.equal(queue.snapshot().payload?.text, "latest");
  assert.equal(calls[1].version, 1);
  calls[1].response.resolve({ id: "draft", payload: calls[1].payload, lockVersion: 2 });
  await Promise.all([saving, navigation]);
  assert.equal(queue.snapshot().dirty, false);
  assert.equal(queue.snapshot().draft?.payload.text, "latest");
  unregister();
});

test("save conflict preserves local text and blocks navigation until explicitly retried", async () => {
  let fail = true;
  let calls = 0;
  const queue = new DraftSaveQueue<Payload, Draft>(async (id, version, payload) => {
    calls++;
    if (fail) throw new Error("version conflict");
    return { id, payload, lockVersion: version + 1 };
  }, (payload) => ({ ...payload }));
  queue.load({ id: "draft", payload: { text: "original" }, lockVersion: 0 });
  queue.edit(() => ({ text: "keep this" }));
  const unregister = registerPendingEdits(() => queue.flush());
  await assert.rejects(flushPendingEdits(), /version conflict/);
  assert.equal(queue.snapshot().payload?.text, "keep this");
  await assert.rejects(flushPendingEdits(), /version conflict/);
  assert.equal(calls, 1);
  fail = false;
  await queue.retry();
  await flushPendingEdits();
  assert.equal(queue.snapshot().dirty, false);
  unregister();
});

test("reverting during an in-flight save is also persisted before leaving", async () => {
  const response = deferred<Draft>();
  const calls: Payload[] = [];
  const queue = new DraftSaveQueue<Payload, Draft>((id, version, payload) => {
    calls.push(payload);
    return calls.length === 1 ? response.promise : Promise.resolve({ id, payload, lockVersion: version + 1 });
  }, (payload) => ({ ...payload }));
  queue.load({ id: "draft", payload: { text: "original" }, lockVersion: 0 });
  queue.edit(() => ({ text: "temporary" }));
  const saving = queue.flush();
  queue.edit(() => ({ text: "original" }));
  response.resolve({ id: "draft", payload: { text: "temporary" }, lockVersion: 1 });
  await saving;
  assert.deepEqual(calls.map((payload) => payload.text), ["temporary", "original"]);
  assert.equal(queue.snapshot().dirty, false);
});

test("leaving before the autosave timer runs flushes the latest payload", async () => {
  let saved: Payload | null = null;
  const queue = new DraftSaveQueue<Payload, Draft>(async (id, version, payload) => {
    saved = payload;
    return { id, payload, lockVersion: version + 1 };
  }, (payload) => ({ ...payload }));
  queue.load({ id: "draft", payload: { text: "original" }, lockVersion: 0 });
  const unregister = registerPendingEdits(() => queue.flush());
  queue.edit(() => ({ text: "just typed" }));
  await flushPendingEdits();
  assert.deepEqual(saved, { text: "just typed" });
  assert.equal(queue.snapshot().dirty, false);
  unregister();
});

test("a completed editor can unregister without blocking other editors", async () => {
  const unregister = registerPendingEdits(async () => { throw new Error("unmounted editor"); });
  unregister();
  await flushPendingEdits();
});
