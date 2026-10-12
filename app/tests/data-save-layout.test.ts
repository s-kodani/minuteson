import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const panel = readFileSync(new URL("../src/data-save.tsx", import.meta.url), "utf8");
test("data settings place the storage action immediately after restore in the shared action row", () => {
  const row = panel.slice(panel.indexOf('className="analysis-actions data-save-primary-actions"'), panel.indexOf('{storage && <><p>会議保存先:'));
  assert.ok(row.indexOf("バックアップを作成") < row.indexOf("バックアップから追加復元"));
  assert.ok(row.indexOf("バックアップから追加復元") < row.indexOf("会議保存先を変更"));
  assert.match(row, /disabled=\{!canAct \|\| !!storage.pendingOperationId\}/);
  assert.match(row, /choose\("relocation", false, event.currentTarget\)/);
  assert.equal((panel.match(/>会議保存先を変更<\/button>/g) ?? []).length, 1);
  assert.doesNotMatch(panel, />状態を再確認<\/button>/);
});
test("removing manual status refresh preserves automatic verification and uncertain-operation recovery", () => {
  const operation = readFileSync(new URL("../src/data-operation.ts", import.meta.url), "utf8");
  assert.match(operation, /window.setInterval\(\(\) => void poll\(\), 1000\)/);
  assert.match(operation, /window.addEventListener\("focus", poll\)/);
  assert.match(panel, /同じ操作の結果を確認/);
  assert.match(panel, /未完了移動を確認/);
});
