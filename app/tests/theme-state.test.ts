import { test } from "node:test";
import assert from "node:assert/strict";
import { assertUiPreferences, resolveTheme, ThemeController, type ThemePreference } from "../src/theme-state.ts";

const snapshot = (theme: ThemePreference = "system", lockVersion = 0) => ({ id: "ui_preferences" as const, payload: { schemaVersion: 1 as const, theme }, lockVersion });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }

test("theme resolves live OS changes only for system and validates exact versioned settings", () => {
  assert.equal(resolveTheme("system", false), "light"); assert.equal(resolveTheme("system", true), "dark");
  assert.equal(resolveTheme("light", true), "light"); assert.equal(resolveTheme("dark", false), "dark");
  assert.deepEqual(assertUiPreferences(snapshot()), snapshot());
  for (const value of [null, {}, { ...snapshot(), extra: true }, { ...snapshot(), id: "other" }, { ...snapshot(), lockVersion: -1 }, { ...snapshot(), lockVersion: 2 ** 53 }, snapshot("dark", 0), { ...snapshot(), payload: { schemaVersion: 2, theme: "system" } }, { ...snapshot(), payload: { schemaVersion: 1, theme: "unknown" } }, { ...snapshot(), payload: { schemaVersion: 1, theme: "system", extra: 1 } }]) assert.throws(() => assertUiPreferences(value));
});

test("theme controller loads and saves confirmed version without changing OS preference", async () => {
  let stored = snapshot(); const writes: unknown[] = [];
  const c = new ThemeController({ read: async () => stored, save: async (version, theme) => { writes.push({ version, theme }); return stored = snapshot(theme, version + 1); } });
  let updates = 0; const unsubscribe = c.subscribe(() => updates++);
  await c.refresh(); assert.equal(writes.length, 0);
  await c.change("dark"); assert.deepEqual(writes, [{ version: 0, theme: "dark" }]);
  assert.deepEqual(c.getSnapshot().snapshot, snapshot("dark", 1)); assert.equal(c.getSnapshot().pendingTheme, null); assert.equal(c.getSnapshot().busy, false);
  await c.refresh(); assert.equal(writes.length, 1); assert.ok(updates > 0); unsubscribe();
});

test("read begun before a save cannot replace the saved theme", async () => {
  const slow = deferred<unknown>(); let reads = 0;
  const c = new ThemeController({ read: () => ++reads === 1 ? Promise.resolve(snapshot()) : slow.promise, save: async () => snapshot("dark", 1) });
  await c.refresh(); const stale = c.refresh(); await c.change("dark"); slow.resolve(snapshot()); await stale;
  assert.deepEqual(c.getSnapshot().snapshot, snapshot("dark", 1)); assert.equal(c.getSnapshot().error, "");
});

test("lost save reply is resolved by authoritative read without another write", async () => {
  let stored = snapshot(); let writes = 0;
  const c = new ThemeController({ read: async () => stored, save: async () => { writes++; stored = snapshot("dark", 1); throw Error("lost reply after commit"); } });
  await c.refresh(); await c.change("dark"); await c.refresh();
  assert.equal(writes, 1); assert.deepEqual(c.getSnapshot().snapshot, stored); assert.equal(c.getSnapshot().pendingTheme, null); assert.equal(c.getSnapshot().error, ""); assert.match(c.getSnapshot().status, /確認しました/);
});

test("failed save keeps requested selection and applies only saved setting until explicit retry", async () => {
  let fail = true; const versions: number[] = []; let stored = snapshot();
  const c = new ThemeController({ read: async () => stored, save: async (version, theme) => { versions.push(version); if (fail) throw Error("disk full"); return stored = snapshot(theme, version + 1); } });
  await c.refresh(); await c.change("dark");
  assert.equal(c.getSnapshot().pendingTheme, "dark"); assert.deepEqual(c.getSnapshot().snapshot, snapshot()); assert.match(c.getSnapshot().error, /保存できません/);
  await c.refresh(); assert.equal(versions.length, 1); assert.equal(c.getSnapshot().pendingTheme, "dark");
  fail = false; await c.change("dark"); assert.deepEqual(versions, [0, 0]); assert.deepEqual(c.getSnapshot().snapshot, snapshot("dark", 1)); assert.equal(c.getSnapshot().pendingTheme, null);
});

test("unconfirmed save cannot be retried until a fresh read succeeds", async () => {
  let readFails = false; let writes = 0; let saveFails = true;
  const c = new ThemeController({ read: async () => { if (readFails) throw Error("read offline"); return snapshot(); }, save: async () => { writes++; if (saveFails) throw Error("uncertain save"); return snapshot("dark", 1); } });
  await c.refresh(); readFails = true; await c.change("dark"); assert.equal(c.getSnapshot().uncertain, true);
  await c.change("dark"); assert.equal(writes, 1); assert.equal(c.getSnapshot().pendingTheme, "dark");
  readFails = false; await c.refresh(); assert.equal(c.getSnapshot().uncertain, false); assert.equal(writes, 1);
  saveFails = false; await c.change("dark"); assert.equal(writes, 2); assert.equal(c.getSnapshot().snapshot?.payload.theme, "dark");
});

test("a later authoritative read settles an unknown committed theme without another save", async () => {
  let stored = snapshot(); let unavailable = false; let writes = 0;
  const c = new ThemeController({ read: async () => { if (unavailable) throw Error("read offline"); return stored; }, save: async () => { writes++; stored = snapshot("dark", 1); unavailable = true; throw Error("lost committed reply"); } });
  await c.refresh(); await c.change("dark");
  assert.equal(c.getSnapshot().uncertain, true); assert.equal(c.getSnapshot().pendingTheme, "dark");
  unavailable = false; await c.refresh(); await c.refresh();
  assert.equal(writes, 1); assert.deepEqual(c.getSnapshot().snapshot, stored);
  assert.equal(c.getSnapshot().uncertain, false); assert.equal(c.getSnapshot().pendingTheme, null);
  assert.equal(c.getSnapshot().error, ""); assert.match(c.getSnapshot().status, /確認しました/);
});

test("mismatched save receipt and stale or altered read responses never replace confirmed state", async () => {
  let stored = snapshot();
  const c = new ThemeController({ read: async () => stored, save: async () => snapshot("light", 1) });
  await c.refresh(); await c.change("dark"); assert.deepEqual(c.getSnapshot().snapshot, snapshot()); assert.equal(c.getSnapshot().pendingTheme, "dark");
  stored = snapshot("dark", 2); await c.refresh(); assert.deepEqual(c.getSnapshot().snapshot, stored);
  stored = snapshot("light", 2); await c.refresh(); assert.equal(c.getSnapshot().uncertain, true); assert.equal(c.getSnapshot().snapshot?.payload.theme, "dark");
  stored = snapshot(); await c.refresh(); assert.equal(c.getSnapshot().snapshot?.lockVersion, 2);
});

test("theme save blocks overlapping requests and reads until actual completion", async () => {
  const slow = deferred<unknown>(); let writes = 0; let reads = 0;
  const c = new ThemeController({ read: async () => { reads++; return snapshot(); }, save: () => { writes++; return slow.promise; } });
  await c.refresh(); const save = c.change("dark"); await c.change("light"); await c.refresh();
  assert.equal(writes, 1); assert.equal(reads, 1); assert.equal(c.getSnapshot().busy, true);
  slow.resolve(snapshot("dark", 1)); await save; assert.equal(c.getSnapshot().busy, false); assert.equal(c.getSnapshot().snapshot?.payload.theme, "dark");
});
