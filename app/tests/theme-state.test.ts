import { test } from "node:test";
import assert from "node:assert/strict";
import { assertUiPreferences, resolveTheme, ThemeController, type ThemePreference, type ColorTheme } from "../src/theme-state.ts";

const snapshot = (theme: ThemePreference = "system", lockVersion = 0, colorTheme: ColorTheme = "monochrome") => ({ id: "ui_preferences" as const, payload: { schemaVersion: 1 as const, theme, colorTheme }, lockVersion });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }

test("missing color defaults to monochrome without changing saved brightness; invalid colors fail closed", () => {
  assert.deepEqual(assertUiPreferences({ ...snapshot("dark", 2), payload: { schemaVersion: 1, theme: "dark" } }), snapshot("dark", 2));
  for (const colorTheme of [null, "blue", "", true, {}, 1]) assert.throws(() => assertUiPreferences({ ...snapshot(), payload: { ...snapshot().payload, colorTheme } }));
  assert.throws(() => assertUiPreferences(snapshot("system", 0, "purple")));
});

test("color and brightness save independently and survive a fresh controller", async () => {
  let stored = snapshot(); const writes: unknown[] = [];
  const bridge = { read: async () => stored, save: async (version: number, theme: ThemePreference, colorTheme: ColorTheme) => {
    writes.push({ theme, colorTheme }); return stored = snapshot(theme, version + 1, colorTheme);
  } };
  const c = new ThemeController(bridge); await c.refresh();
  await c.changeColor("purple"); await c.change("dark"); await c.changeColor("monochrome");
  assert.deepEqual(writes, [{theme:"system",colorTheme:"purple"},{theme:"dark",colorTheme:"purple"},{theme:"dark",colorTheme:"monochrome"}]);
  const restarted = new ThemeController(bridge); await restarted.refresh();
  assert.deepEqual(restarted.getSnapshot().snapshot, snapshot("dark", 3));
  assert.equal(restarted.getSnapshot().notice, null);
});

test("color retry merges only requested fields with a newer saved brightness", async () => {
  let stored = snapshot(), fail = true;
  const c = new ThemeController({ read: async () => stored, save: async (version, theme, colorTheme) => {
    if (fail) throw Error("save failed"); return stored = snapshot(theme, version + 1, colorTheme);
  } });
  await c.refresh(); await c.changeColor("purple");
  assert.equal(c.getSnapshot().snapshot?.payload.colorTheme, "monochrome");
  assert.equal(c.getSnapshot().pendingPreferences?.colorTheme, "purple");
  stored = snapshot("dark", 2); await c.refresh(); fail = false; await c.retry();
  assert.deepEqual(c.getSnapshot().snapshot, snapshot("dark", 3, "purple"));
});

test("matching brightness alone cannot resolve an unknown color save", async () => {
  let stored = snapshot(), unavailable = false, writes = 0;
  const c = new ThemeController({ read: async () => { if (unavailable) throw Error("offline"); return stored; }, save: async () => { writes++; unavailable = true; throw Error("lost response"); } });
  await c.refresh(); await c.changeColor("purple");
  await c.change("dark"); assert.equal(writes, 1);
  unavailable = false; await c.refresh();
  assert.equal(c.getSnapshot().pendingPreferences?.colorTheme, "purple");
  assert.equal(c.getSnapshot().snapshot?.payload.colorTheme, "monochrome");
  stored = snapshot("system", 1, "purple"); await c.refresh();
  assert.equal(c.getSnapshot().pendingPreferences, null); assert.equal(writes, 1);
});

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

test("routine theme reads emit no result; explicit saves have distinct acknowledgement identities", async () => {
  let stored = snapshot(); let writes = 0;
  const c = new ThemeController({ read: async () => stored, save: async (version, theme) => { writes++; return stored = snapshot(theme, version + (theme === stored.payload.theme ? 0 : 1)); } });
  await c.refresh(); await c.refresh(); assert.equal(c.getSnapshot().notice, null);
  await c.change("dark"); const first = c.getSnapshot().notice!;
  assert.equal(first.level, "success"); assert.match(first.message, /保存しました/);
  await c.refresh(); assert.equal(c.getSnapshot().notice, first);
  c.clearNotice(first.id); await c.refresh(); assert.equal(c.getSnapshot().notice, null);
  await c.change("light"); const second = c.getSnapshot().notice!;
  assert.ok(second.id > first.id); c.clearNotice(first.id); assert.equal(c.getSnapshot().notice, second);
  assert.equal(writes, 2); assert.equal(c.getSnapshot().error, "");
});

test("lost theme acknowledgement notifies a read-confirmed result once and never saves again", async () => {
  let stored = snapshot(); let unavailable = false; let writes = 0;
  const c = new ThemeController({ read: async () => { if (unavailable) throw Error("offline"); return stored; }, save: async () => { writes++; stored = snapshot("dark", 1); unavailable = true; throw Error("lost reply"); } });
  await c.refresh(); await c.change("dark"); assert.equal(c.getSnapshot().notice, null);
  unavailable = false; await c.refresh(); const notice = c.getSnapshot().notice!;
  assert.equal(notice.level, "info"); assert.match(notice.message, /確認しました/);
  await c.refresh(); assert.equal(c.getSnapshot().notice, notice);
  c.clearNotice(notice.id); await c.refresh(); assert.equal(c.getSnapshot().notice, null); assert.equal(writes, 1);
});

test("failed theme save and failed reload never notify success or discard an error on dismissal", async () => {
  let readFails = false;
  const c = new ThemeController({ read: async () => { if (readFails) throw Error("offline"); return snapshot(); }, save: async () => { throw Error("disk full"); } });
  await c.refresh(); await c.change("dark"); c.clearNotice(1);
  assert.equal(c.getSnapshot().notice, null); assert.equal(c.getSnapshot().pendingTheme, "dark"); assert.match(c.getSnapshot().error, /disk full/);
  readFails = true; await c.reload(); assert.equal(c.getSnapshot().notice, null); assert.equal(c.getSnapshot().uncertain, true);
  readFails = false; await c.reload(); assert.equal(c.getSnapshot().notice?.level, "info");
});

test("an older explicit reload cannot announce a newer in-flight theme save as read-confirmed", async () => {
  const read = deferred<unknown>(), save = deferred<unknown>(); let reads = 0;
  const c = new ThemeController({ read: () => ++reads === 1 ? Promise.resolve(snapshot()) : read.promise, save: () => save.promise });
  await c.refresh(); const reload = c.reload(); const writing = c.change("dark");
  read.resolve(snapshot()); await reload; assert.equal(c.getSnapshot().notice, null); assert.equal(c.getSnapshot().busy, true);
  save.resolve(snapshot("dark", 1)); await writing; assert.equal(c.getSnapshot().notice?.level, "success");
});
