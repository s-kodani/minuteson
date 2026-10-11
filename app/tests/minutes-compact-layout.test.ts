import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");

test("Markdown export is a labeled icon retaining the same request, busy guard and focus target", () => {
  const panel=source("minutes.tsx"),icon=source("action-icon.tsx");
  assert.match(panel,/<IconButton label=\{pendingExport.current \? "前回の書き出し結果を確認" : "Markdownを書き出す"\} icon="download" disabled=\{disabled\} onClick=\{event => void writeMarkdown\(event.currentTarget\)\}/);
  assert.match(icon,/name === "download"/);
  assert.match(panel,/exportFocusReturn.current/);
  assert.match(panel,/保存済みの手動編集を含むことを示します/);
  assert.match(panel,/currentDraft && "· 編集あり"/);
  assert.match(panel,/role="status">元データ更新あり/);
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
