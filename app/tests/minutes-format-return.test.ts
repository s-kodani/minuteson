import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const text = readFileSync(new URL("../src/minutes-editor.tsx", import.meta.url), "utf8");
const source = ts.createSourceFile("minutes-editor.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
function declaration(name: string) {
  let result: ts.VariableDeclaration | undefined;
  const visit = (node: ts.Node) => { if (ts.isVariableDeclaration(node) && node.name.getText(source) === name) result = node; ts.forEachChild(node, visit); };
  visit(source); assert.ok(result); return result.getText(source);
}
test("successful modal structure changes close options after choosing a surviving editor address", () => {
  const structural = declaration("structural");
  assert.ok(structural.indexOf("if (!edit(action)) return false") < structural.indexOf("setFormatOpen(false)"));
  assert.ok(structural.indexOf("documentAddress(blocks, requested)") < structural.indexOf("setFormatOpen(false)"));
  assert.match(structural, /if \(formatOpen\) \{\s*focusAfterOptions.current = true;\s*setFormatOpen\(false\);\s*\}/);
  assert.match(structural, /documentPageForAddress\(blocks,/);
});
test("no native dialog is bypassed to restore the rich field after options or confirmation", () => {
  assert.match(text, /if \(confirm.pending\) \{ focusAfterConfirmation.current = true; return; \}/);
  assert.match(text, /if \(document.querySelector\("dialog\[open\]"\) \|\| focusAddress.current === null\) return/);
  assert.match(text, /const frame = requestAnimationFrame\(focus\)/);
  assert.match(text, /if \(afterOptions && document.activeElement !== formatTrigger.current && document.activeElement !== document.body\) return/);
});
