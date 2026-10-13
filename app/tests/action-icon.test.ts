import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import ts from "typescript";
const source = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");

test("icon buttons retain human labels in hover help and accessibility without decorative SVG names", () => {
  const icon = source("action-icon.tsx");
  assert.match(icon, /aria-label=\{label\} title=\{label\}/);
  assert.match(icon, /aria-hidden="true" focusable="false"/);
  assert.match(icon, /\{\.\.\.props\} ref=\{ref\}/);
  assert.match(icon, /type=\{props.type \?\? "button"\}/);
  for (const name of ["close", "edit", "play", "pause", "restart", "more", "refresh", "undo", "plus", "previous", "next"]) assert.ok(icon.includes(`name === "${name}"`));
});

test("audio icons keep playback, preparation and lock guards and descriptive state labels", () => {
  const player = source("transcript-audio.tsx"), meeting = source("meeting-audio.tsx");
  assert.match(player, /label=\{preparing \? "再生準備を取り消す" : snapshot.playing \? "一時停止" : "再生"\} icon=\{preparing \? "close" : snapshot.playing \? "pause" : "play"\} onClick=\{preparing \? clear : playFromBar\} disabled=\{locked\}/);
  assert.match(player, /label="先頭へ" icon="restart" onClick=\{\(\) => \{ player.pause\(\); player.seek\(0\); \}\} disabled=\{!info \|\| preparing \|\| locked\}/);
  assert.match(player, /label="再生を閉じる" icon="close" onClick=\{clear\} disabled=\{locked\}/);
  assert.match(meeting, /<OptionsModal label="再生設定" icon="more"/);
  assert.match(source("options-modal.tsx"), /aria-haspopup="dialog" aria-controls=\{scope\?\.id\?\?id\} aria-expanded=\{open && !hidden\}/);
});

test("modal close and icon entries preserve focus, controls, attention and blocking contracts", () => {
  const modal = source("common-modal.tsx"), options = source("options-modal.tsx");
  assert.match(modal, /title=\{customDismissText \?\? label\} data-dialog-cancel disabled=\{effectiveBlocked\} onClick=\{dismiss\}><ActionIcon name="close"/);
  assert.match(options, /title=\{triggerLabel === "⋯" \? label : triggerLabel\}/);
  assert.match(options, /aria-controls=\{scope\?\.id\?\?id\} aria-expanded=\{open && !hidden\}/);
  assert.match(options, /event.currentTarget.focus\(\{preventScroll:true\}\);change\(true\)/);
  assert.match(options, /aria-label="要確認"/);
});


test("shared icon controls remain square and centered at each action row size", () => {
  const css = source("prototype-alignment.css");
  assert.match(css, /:root button.icon-button \{ display:inline-flex; align-items:center; justify-content:center; flex:none;/);
  assert.match(css, /width:var\(--action-height,40px\); height:var\(--action-height,40px\)/);
  assert.match(css, /padding:0; line-height:1; position:relative/);
  assert.match(css, /\.action-icon \{ display:block; flex:none; pointer-events:none/);
});


test("playback speed never shrinks into the timestamp and matches adjacent icon heights", () => {
  const css = source("prototype-alignment.css");
  assert.match(css, /\.meeting-playback-dock \.audio-controls>label \{[^}]*flex:none; flex-wrap:nowrap;/);
  assert.match(css, /\.meeting-playback-dock \.audio-controls \{ flex:1 1 320px; flex-wrap:nowrap; gap:8px;/);
  assert.match(css, /\.audio-controls select \{ box-sizing:border-box; height:var\(--action-height,40px\); min-height:var\(--action-height,40px\);/);
  assert.match(css, /line-height:var\(--action-line-height,20px\);/);
});

test("ordinary refresh, pause and page navigation labels do not remain as text-only buttons", () => {
  const directory = new URL("../src/", import.meta.url);
  const remaining: string[] = [];
  const icons = new Set<string>();
  for (const file of readdirSync(directory).filter(name => name.endsWith(".tsx"))) {
    const text = readFileSync(new URL(file,directory),"utf8");
    const parsed = ts.createSourceFile(file,text,ts.ScriptTarget.Latest,true,ts.ScriptKind.TSX);
    const ordinary = (label: string) => /再読込$|一時停止$|^(?:(?:モデル|音声)対象の)?(?:前の|次の)|^(?:会議一覧へ|議事録の元の位置へ)戻る$/.test(label);
    function visit(node: ts.Node) {
      if (ts.isJsxElement(node) && node.openingElement.tagName.getText(parsed)==="button" && node.children.every(ts.isJsxText)) {
        const label=node.children.map(child=>(child as ts.JsxText).text).join("").trim();
        if (ordinary(label)) remaining.push(`${file}: ${label}`);
      }
      if (ts.isJsxSelfClosingElement(node) && node.tagName.getText(parsed)==="IconButton") {
        const label=node.attributes.properties.find(p=>ts.isJsxAttribute(p)&&p.name.getText(parsed)==="label");
        if (label && ts.isJsxAttribute(label) && label.initializer && ts.isStringLiteral(label.initializer) && ordinary(label.initializer.text)) icons.add(`${file}:${label.initializer.text}`);
      }
      ts.forEachChild(node,visit);
    }
    visit(parsed);
  }
  assert.deepEqual(remaining,[]);
  for (const entry of ["minutes.tsx:議事録を再読込","recording-workspace.tsx:会議一覧へ戻る"]) assert.ok(icons.has(entry),entry);
});
