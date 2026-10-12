import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
const main=readFileSync(new URL("../src/main.tsx",import.meta.url),"utf8");
test("meeting title editing has its own guarded modal instead of management sections",()=>{
  assert.doesNotMatch(main,/<h2>録音セッション|<h3>会議情報|className="rename-form"/);
  assert.match(main,/meeting-title-row.*label="会議名を変更".*icon="edit"/);
  assert.match(main,/open=\{renameOpen && !!selectedMeeting\} blocked=\{meetingBusy\}/);
  assert.match(main,/expectedLockVersion: selectedMeeting.lockVersion/);
  assert.match(main,/!renameOpen \|\| meetingBusy \|\| writeLocked \|\| !renameTitle.trim\(\)/);
});
test("navigation has no routine progress toast and settings gear is 28px",()=>{
  assert.doesNotMatch(main,/navigation-progress|編集の保存を確認中|操作を実行中/);
  assert.doesNotMatch(main,/navigationBusy && <(?:p|span) role="status"/);
  assert.match(main,/settings-nav-glyph.*ActionIcon name="gear"/);
  const css=readFileSync(new URL("../src/prototype-alignment.css",import.meta.url),"utf8");
  assert.match(css,/\.settings-nav-glyph \{[^}]*align-items: center; justify-content: center; flex: 0 0 28px; height: 28px;/);
  assert.match(css,/\.settings-nav-glyph \.action-icon \{ width: 28px; height: 28px;/);
  const theme=readFileSync(new URL("../src/theme.tsx",import.meta.url),"utf8");
  assert.doesNotMatch(theme,/本文と再生の表示設定|保存済み表示設定を再読込/);
});
