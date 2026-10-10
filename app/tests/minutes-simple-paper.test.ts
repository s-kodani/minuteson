import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
test("single paper editing keeps the reader and fixed references mounted", () => {
  const editor = source("minutes-editor.tsx"), reader = source("minutes-document.tsx");
  assert.match(editor, /readerHidden=\{editing\}/);
  assert.match(reader, /<div hidden=\{full \|\| readerHidden\}>/);
  assert.match(reader, /panelHost && createPortal\(references,panelHost\)/);
  assert.match(reader, /<details className="minutes-outline-tools"/);
  assert.match(editor, /className="minutes-primary-toolbar"[^\n]*hidden=\{full\}/);
  assert.match(editor, /verify=\{disabled \|\| failure.current \|\| compositions.current.size/);
  assert.match(source("prototype-alignment.css"), /\.minutes-editor \.minutes-document-layout article h1 \{ font-size:1\.6em;/);
});
test("editing evidence activation retains atomic references and blocks unknown or locked targets", () => {
  const field = source("minutes-rich-field.tsx"), reader = source("minutes-document.tsx");
  assert.match(field, /chip.contentEditable = "false"/);
  assert.match(field, /chip.setAttribute\("role", "button"\)/);
  assert.match(field, /known.current.has\(chip.dataset.evidenceId!\)/);
  assert.match(field, /\["Enter", " "\].includes\(event.key\)/);
  assert.match(field, /event.preventDefault\(\); event.stopPropagation\(\)/);
  assert.match(reader, /if \(locked \|\| full \|\| !active \|\| !content.evidence.some/);
  assert.match(reader, /reveal\(saved.panelOpener\)/);
  assert.match(reader, /setSelected\(null\); panelHandle\?\.current\?\.dismiss\(\)/);
});
test("optional wide panel collapse preserves one reparented portal and safe focus", () => {
  const panel = source("workspace-panel.tsx");
  assert.match(panel, /collapsible = false/);
  assert.match(panel, /destination.append\(content\)/);
  assert.match(panel, /hidden=\{narrow \|\| collapsible && !open\}/);
  assert.match(panel, /document.querySelector\("dialog\[open\]"\)/);
  assert.match(panel, /!target.closest\("\[hidden\]"\)/);
  assert.match(source("minutes-editor.tsx"), /<WorkspacePanel collapsible content=\{referenceMount\}/);
});
