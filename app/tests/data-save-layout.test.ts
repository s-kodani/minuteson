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
test("backup and restore notices describe current saved documents without removed draft or evidence UI", () => {
  assert.match(panel, /保存済み議事録・用語を保存します/);
  assert.match(panel, /保存済み文書と音声の欠落情報/);
  assert.doesNotMatch(panel, /本文・根拠|根拠・下書き|議事録・下書き/);
});
test("removing manual status refresh preserves automatic verification and uncertain-operation recovery", () => {
  const operation = readFileSync(new URL("../src/data-operation.ts", import.meta.url), "utf8");
  assert.match(operation, /window.setInterval\(\(\) => void poll\(\), 1000\)/);
  assert.match(operation, /window.addEventListener\("focus", poll\)/);
  assert.match(panel, /同じ操作の結果を確認/);
  assert.match(panel, /未完了移動を確認/);
});
test("data availability must be read for the current snapshot revision before its next phase", () => {
  const operation = readFileSync(new URL("../src/data-operation.ts", import.meta.url), "utf8");
  assert.match(operation, /if \(value.revision !== current.current\?\.revision\) setAvailability\(null\)/);
  assert.match(operation, /epoch === readEpoch.current && current.current\?\.revision === next.revision/);
  assert.match(operation, /invoke<DataAvailability>\("get_data_operation_availability"\)/);
});
test("historical notification IO stays paused across data selection and ready handoff", () => {
  const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
  const start = main.indexOf("<ArchivedInputNotices");
  const notice = main.slice(start, main.indexOf("/>", start));
  assert.match(notice, /paused=\{/);
  assert.match(notice, /dataPending/);
  assert.match(notice, /dataOperation.snapshot\?\.phase===\"ready\"/);
  assert.match(notice, /dataOperation.snapshot\?\.busy/);
});
