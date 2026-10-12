import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");

test("Markdown export is a labeled icon retaining the same request, busy guard and focus target", () => {
  const panel=source("minutes.tsx"), editor=source("minutes-markdown-editor.tsx");
  assert.match(panel,/className="plain-minutes-primary-actions"/);
  assert.match(editor,/label=\{editing\?"議事録のプレビューを表示":"議事録を編集"\}/);
  assert.match(editor,/state\.saving\?"保存中":state\.dirty\?"未保存"/);
  assert.doesNotMatch(panel,/現在版 #|元データ更新あり|根拠/);
});

test("formatting detail entry lives with the basic formatting icons, not in the meeting header", () => {
  const text=source("minutes-editor.tsx");
  const parsed=ts.createSourceFile("minutes-editor.tsx",text,ts.ScriptTarget.Latest,true,ts.ScriptKind.TSX);
  let found=0;
  function visit(node:ts.Node) {
    if(ts.isJsxSelfClosingElement(node)&&node.getText(parsed).includes('className="minutes-format-actions-host"')) {
      found++;
      const parent=node.parent;
      assert.ok(ts.isJsxElement(parent));
      assert.ok(parent.openingElement.getText(parsed).includes('className="minutes-primary-toolbar"'));
      assert.match(node.getText(parsed),/ref=\{setFormatActionsHost\}/);
    }
    ts.forEachChild(node,visit);
  }
  visit(parsed);
  assert.equal(found,1);
  assert.match(text,/triggerHost=\{formatActionsHost\} triggerRef=\{formatTrigger\}/);
  assert.match(text,/focusAddress.current/);
});

test("save states remain visible and named, including dirty, busy and error states", () => {
  assert.match(source("minutes-editor.tsx"),/role="status" aria-label="議事録の保存状態" className="minutes-save-status">\{saved.error \? "保存要確認" : saved.saving \? "保存中" : saved.dirty \? "未保存" : "保存済み"\}/);
  assert.match(source("minutes-editor.tsx"),/label="議事録の保存に失敗"/);
  const css=source("prototype-alignment.css");
  assert.match(css,/\.minutes-save-status \{[^}]*white-space:nowrap;/);
  assert.match(css,/\.main-column:has\(\.meeting-detail-status\)>\.topbar>\.header-recording \{ width:auto; min-width:0; justify-content:flex-end; \}/);
  assert.match(css,/\.meeting-detail-content \.minutes-card \{ padding:12px 16px; gap:8px; \}/);
  assert.match(css,/\.meeting-detail-content \.minutes-document-status \{ gap:6px; font-size:11px; \}/);
  assert.doesNotMatch(css,/\.minutes-save-status\s*\{[^}]*(?:display:none|height:0|visibility:hidden)/);
});
