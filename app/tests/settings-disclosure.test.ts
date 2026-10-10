import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
test("settings disclosures retain mounted children and attention after it clears", () => {
  const view = source("settings-disclosure.tsx");
  assert.match(view, /const expanded = open \|\| attention/);
  assert.match(view, /if \(attention\) setOpen\(true\)/);
  assert.match(view, /disabled=\{attention\}/);
  assert.match(view, /hidden=\{!expanded\}>\{children\}/);
  assert.doesNotMatch(view, /expanded &&/);
  assert.match(source("prototype-alignment.css"), /\.settings-disclosure-body\[hidden\] \{ display:none; \}/);
});
test("normal settings keep primary theme and model summary but fold diagnostics", () => {
  const main = source("main.tsx"), theme = source("theme.tsx"), models = source("model-inspection.tsx");
  assert.match(main, /<ThemeSettingsPanel compact locked/);
  assert.match(main, /<ModelInspectionPanel compact locked/);
  assert.ok(theme.indexOf('<ThemeSelection locked={locked} />') < theme.indexOf('<SettingsDisclosure label="本文と再生の表示設定"'));
  assert.match(theme, /attention=\{!!\(state.error \|\| state.busy \|\| state.uncertain \|\| state.pendingPreferences\)\}/);
  assert.match(models, /standard \? profileConfigurationLabel\(standard\)/);
  assert.match(models, /attention=\{recordsAttention \|\| !!report\}/);
  assert.match(models, /entry.bodyRemoved !== true && entry.compatibility\?\.state !== "succeeded"/);
  assert.match(models, /<\/SettingsDisclosure>\s*\{deletionReview && <CommonModal/);
});
test("download recovery and confirmation are not hidden with the form or catalog", () => {
  const view = source("model-download.tsx");
  assert.match(view, /<\/SettingsDisclosure>\s*\{!!busy && <p role="status"/);
  assert.match(view, /<\/SettingsDisclosure>\s*<CommonModal open=\{!!modal\}/);
  assert.match(view, /entry.state !== "completion_recorded"/);
  assert.match(view, /<SettingsDisclosure label="取得定義からダウンロード"[^\n]*pending \|\| modal \|\| review \|\| receipt \|\| error/);
});
