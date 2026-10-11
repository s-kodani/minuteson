import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { quietMaintenance, maintenanceReadRevision } from "../src/maintenance-visibility.ts";

const idle = { compact: true, connected: true, supported: true, listed: true, itemCount: 0, attention: false };
test("compact maintenance hides only a verified empty, supported and connected state", () => {
  assert.equal(quietMaintenance(idle), true);
  for (const patch of [{ compact: false }, { connected: false }, { supported: false }, { listed: false }, { itemCount: 1 }, { itemCount: NaN }, { itemCount: -1 }, { attention: true }]) {
    assert.equal(quietMaintenance({ ...idle, ...patch }), false, JSON.stringify(patch));
  }
});
test("compact presentation does not unmount controllers, notifications or confirmation modals", () => {
  const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
  assert.equal((main.match(/compact=\{!settingsManagementOpen\}/g) ?? []).length, 5);
  assert.match(main, /<OptionsModal[^\n]*open=\{settingsManagementOpen\} onOpenChange=\{setSettingsManagementOpen\}/);
  for (const file of ["meeting-deletion", "model-body-deletion", "audio-deletion", "recovery-panel", "partial-recording-panel"]) {
    const source = readFileSync(new URL(`../src/${file}.tsx`, import.meta.url), "utf8");
    assert.match(source, /compact = false/);
    assert.match(source, /quietMaintenance\(/);
    assert.match(source, /attention:.*(?:active|s\?\.kind)/);
    assert.match(source, /<CommonModal/);
    assert.doesNotMatch(source, /if\s*\(quiet\)\s*return/);
  }
});
test("diagnostics refresh after all terminal outcomes, not every idle or busy poll", () => {
  assert.equal(maintenanceReadRevision(null), -1);
  for (const phase of ["completed", "failed", "cancelled", "attention"]) {
    assert.equal(maintenanceReadRevision({ phase, busy: false, revision: 9 }), 9);
    assert.equal(maintenanceReadRevision({ phase, busy: true, revision: 9 }), -1);
  }
  for (const phase of ["idle", "ready", "verifying", "copying", "cancelling"]) {
    assert.equal(maintenanceReadRevision({ phase, busy: false, revision: 9 }), -1);
  }
  const source = readFileSync(new URL("../src/recovery-panel.tsx", import.meta.url), "utf8");
  assert.match(source, /\[visible, maintenanceReadRevision\(s\)\]/);
});
