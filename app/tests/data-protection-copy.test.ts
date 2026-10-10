import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// These listings also carry a storage-relocation fence with no deletion items.
// Keep the copy check separate from controller/permission tests: it must not
// change when an operation is allowed, only what that shared fence claims.
for (const file of ["meeting-deletion", "model-body-deletion", "audio-deletion"]) {
  test(`${file} reports the shared write fence without inventing a deletion or recovery`, () => {
    const source = readFileSync(new URL(`../src/${file}.tsx`, import.meta.url), "utf8");
    const notice = source.split("\n").find(line => line.includes('{needed && <p role="alert">'));
    assert.ok(notice);
    assert.match(notice, /通常書込みを停止しています。\{c\.listing\?\.reason\}/);
    assert.doesNotMatch(notice, /未完了削除|確認待ちの.*削除|既に消した/);
    assert.doesNotMatch(source, /保留した起動時回復を行うため|モデル削除確認後に終了する|音声削除確認後に終了する/);
    assert.match(source, />終了する（再起動は手動）<\/button>/);
  });
}
