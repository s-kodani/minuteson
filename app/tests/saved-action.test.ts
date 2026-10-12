import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runAfterSaving, type SavedActionPhase } from "../src/saved-action.ts";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(yes => { resolve = yes; });
  return { promise, resolve };
}

test("save and operation waits have distinct phases and remain locked until the operation finishes", async () => {
  const saving = deferred(), operation = deferred();
  const phases: SavedActionPhase[] = [];
  let started = false, finished = false;
  const running = runAfterSaving(() => saving.promise, () => {
    started = true; return operation.promise;
  }, phase => phases.push(phase)).then(() => { finished = true; });
  assert.deepEqual(phases, ["saving"]);
  assert.equal(started, false);
  saving.resolve(); await Promise.resolve();
  assert.deepEqual(phases, ["saving", "working"]);
  assert.equal(started, true); assert.equal(finished, false);
  operation.resolve(); await running;
  assert.deepEqual(phases, ["saving", "working", "idle"]);
  assert.equal(finished, true);
});

test("failed saving preserves the error and never starts the operation", async () => {
  const failure = Error("save conflict"), phases: SavedActionPhase[] = [];
  let calls = 0;
  await assert.rejects(runAfterSaving(async () => { throw failure; }, () => { calls++; },
    phase => phases.push(phase)), error => error === failure);
  assert.equal(calls, 0);
  assert.deepEqual(phases, ["saving", "idle"]);
});

test("failed operation returns to idle without hiding its original error", async () => {
  const failure = Error("operation failed"), phases: SavedActionPhase[] = [];
  await assert.rejects(runAfterSaving(async () => {}, async () => { throw failure; },
    phase => phases.push(phase)), error => error === failure);
  assert.deepEqual(phases, ["saving", "working", "idle"]);
});

test("synchronous navigation still passes the save barrier and completes", async () => {
  const events: string[] = [];
  await runAfterSaving(async () => { events.push("flush"); }, () => { events.push("navigate"); },
    phase => events.push(phase));
  assert.deepEqual(events, ["saving", "flush", "working", "navigate", "idle"]);
});

test("workspace reports the actual phase without removing busy and duplicate-operation guards", () => {
  const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
  assert.match(main, /navigationBusy = navigationPhase !== "idle"/);
  assert.match(main, /if \(navigating.current\) return;\s*navigating.current = true/);
  assert.match(main, /await runAfterSaving\(\(\) => flushPendingEdits\(reason\), action, setNavigationPhase\)/);
  assert.match(main, /reason: "leave" \| "tab" = "leave"/);
  assert.doesNotMatch(main, /編集の保存を確認中|操作を実行中|navigation-progress/);
  assert.doesNotMatch(main, /setNavigationBusy/);
});
