import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { SettingsModalState, type SettingsView } from "../src/settings-modal-state.ts";
const view = (id: string, open = false): SettingsView => ({ id, open, title: id, className: id, dismiss: () => {}, entry: () => null });

test("closed settings registration never opens a native owner or executes work", () => {
  const state = new SettingsModalState();
  state.update(view("audio")); state.update(view("edit"));
  assert.equal(state.getSnapshot().active, null);
});
test("successive sibling settings reuse presentation ownership without keeping stale entries", () => {
  const state = new SettingsModalState(), audio = view("audio"), edit = view("edit");
  for (let i = 0; i < 3; i++) {
    state.update({ ...audio, open: true }); assert.equal(state.getSnapshot().active?.id, "audio");
    state.update(audio); assert.equal(state.getSnapshot().active, null);
    state.update({ ...edit, open: true }); assert.equal(state.getSnapshot().active?.id, "edit");
    state.update(edit); assert.equal(state.getSnapshot().active, null);
  }
});
test("nested settings close only their owner and retain the actual parent entry", () => {
  const state = new SettingsModalState(), parent = view("parent", true), child = view("child", true);
  state.update(parent); state.update(child);
  assert.equal(state.getSnapshot().active, child);
  state.update({ ...child, open: false });
  assert.equal(state.getSnapshot().active, parent);
  assert.equal(state.getSnapshot().returning, child.entry);
});
test("parent updates do not steal an open child and removing a child resumes a surviving parent", () => {
  const state = new SettingsModalState(), parent = view("parent", true), child = view("child", true);
  state.update(parent); state.update(child);
  const changed = { ...parent, title: "retained settings" };
  state.update(changed); assert.equal(state.getSnapshot().active, child);
  state.remove(child.id); assert.equal(state.getSnapshot().active, changed);
  assert.equal(state.getSnapshot().returning, child.entry);
});
test("identical descriptors do not publish recursive renders and cleanup is idempotent", () => {
  const state = new SettingsModalState(), descriptor = view("edit", true);
  let calls = 0; const unsubscribe = state.subscribe(() => calls++);
  state.update(descriptor); const snapshot = state.getSnapshot();
  state.update({ ...descriptor }); assert.equal(state.getSnapshot(), snapshot); assert.equal(calls, 1);
  state.remove(descriptor.id); state.remove(descriptor.id); assert.equal(calls, 2);
  unsubscribe(); state.update(descriptor); assert.equal(calls, 2);
});
test("a hidden parent removed during child settings never reopens on child dismissal", () => {
  const state = new SettingsModalState(), parent = view("parent", true), child = view("child", true);
  state.update(parent); state.update(child); state.update({ ...parent, open: false });
  assert.equal(state.getSnapshot().active, child);
  state.update({ ...child, open: false }); assert.equal(state.getSnapshot().active, null);
});
test("scoped settings keep base/replacement inputs mounted without moving hosts or running save commands", () => {
  const source = name => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
  const scope = source("settings-modal-scope.tsx"), options = source("options-modal.tsx");
  assert.equal((scope.match(/<CommonModal/g) ?? []).length, 1);
  assert.match(scope, /persistentReplacement=\{<div ref=\{setReplacement\}/);
  assert.match(scope, /restoreTarget=\{owner\?\.entry\} replacement=\{owner\?\.replacement\}/);
  assert.match(scope, /previous.current\?\.replacement\?\.key !== current.replacement.key/);
  assert.match(scope, /previous.current.id !== current.id \|\| enteringReplacement/);
  assert.match(scope, /enteringReplacement \|\| replacementReady/);
  assert.match(scope, /previous.current.replacement.blocked && !current.replacement.blocked/);
  assert.match(scope, /if \(!replacementReady \|\| !validFocus\)/);
  assert.match(scope,/requestAnimationFrame\(\(\) =>/);
  assert.match(scope,/nested externalContent data-settings-modal/);
  assert.match(scope,/topOpenModal\(\) !== element \|\| live\?\.id !== current.id/);
  assert.match(scope,/active !== document.body && active !== element/);
  assert.match(options, /scope.body&&createPortal/); assert.match(options, /scope.replacement&&createPortal/);
  assert.match(options, /hidden=\{active\?\.active\?\.id!==id\}/);
  assert.doesNotMatch(options,/settings-modal-view \$\{className\}/);
  assert.match(source("main.tsx"), /<SettingsModalScope><MainWindow \/><\/SettingsModalScope>/);
  assert.doesNotMatch(scope, /invoke\(|\.flush\(|\.append(?:Child)?\(|setTimeout/);
});
test("visible-control shared settings driver covers retained fields and confirmation cancellation", () => {
  const driver = readFileSync(new URL("./settings-modal-browser-check.js", import.meta.url),"utf8");
  const runner = readFileSync(new URL("./run-browser-checks.mjs", import.meta.url),"utf8");
  assert.match(runner,/\["shared-settings", "settings-modal-browser-fixture.tsx", "settings-modal-browser-check.js"\]/);
  assert.match(driver,/i<2/); assert.match(driver,/実行回数 0/);
  assert.match(driver,/dialog\[open\]/); assert.match(driver,/確認をキャンセル/);
  assert.doesNotMatch(driver,/window\.|monBrowserFixture|合成変更を実行.*(?:click|press)/);
  assert.doesNotThrow(()=>new Function(`return (${driver})`));
});
test("async review fixture checks both lost focus repair and valid focus retention", () => {
  const driver = readFileSync(new URL("./settings-modal-browser-check.js", import.meta.url),"utf8");
  const fixture = readFileSync(new URL("./settings-modal-browser-fixture.tsx", import.meta.url),"utf8");
  assert.match(driver,/keepFocus of \[false,true\]/);
  assert.match(driver,/review readiness lost or stole focus/);
  assert.match(fixture,/blocked:reviewBusy/);
  assert.match(fixture,/disabled=\{!reviewBusy\}/);
  assert.match(fixture,/合成の読込を完了（焦点保持）/);
});
