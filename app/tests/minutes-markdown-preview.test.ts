import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";

// Execute the actual TSX component and its actual Markdown dependencies.
// Transpile in memory only; no browser, temp bundle or replacement renderer.
const nativeRequire=createRequire(import.meta.url);
const modules=new Map<string,any>();
function componentModule(name:string):any {
  if(modules.has(name))return modules.get(name);
  const extension=name==="minutes-markdown-state"?"ts":"tsx";
  const source=readFileSync(new URL(`../src/${name}.${extension}`,import.meta.url),"utf8");
  const compiled=ts.transpileModule(source,{compilerOptions:{jsx:ts.JsxEmit.ReactJSX,module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,esModuleInterop:true}}).outputText;
  const module={exports:{}};
  const require=(id:string)=>id.startsWith("./")?componentModule(id.slice(2)):nativeRequire(id);
  new Function("require","module","exports",compiled)(require,module,module.exports);
  modules.set(name,module.exports);return module.exports;
}
const {MinutesMarkdownPreview}=componentModule("minutes-markdown-preview");
function render(text:string,durationUs:number|null=10_000_000,locked=false) {
  return renderToStaticMarkup(createElement(MinutesMarkdownPreview,{text,durationUs,locked,onPlay:()=>{throw Error("Rendering must not play audio");}}));
}
test("actual Markdown preview supports headings emphasis lists fenced code tables and task lists",()=>{
  const html=render("# 点検日\n\n**決定** *未確認* ~~旧記載~~\n\n- 項目\n- [x] 作業\n\n```js\nalert(1)\n```\n\n| 項目 | 状態 |\n| --- | --- |\n| 点検 | 未確認 |\n");
  for(const part of ["<h1>点検日</h1>","<strong>決定</strong>","<em>未確認</em>","<del>旧記載</del>","<ul", "<li>","<pre><code", "<table>","<td>点検</td>","disabled=\"\""])assert.ok(html.includes(part),part);
});
test("scripts raw HTML dangerous links and all image sources are inert in the actual renderer",()=>{
  const html=render("<script>alert(1)</script>\n\n<img src=\"https://x/collect\" onerror=\"alert(2)\">\n\n[危険](javascript:alert(3)) [外部](https://example.com)\n\n![外部画像](https://x/image) ![ローカル](file:///secret) ![データ](data:image/svg+xml;base64,AAA)\n");
  assert.doesNotMatch(html,/<script|<img|<iframe|\bsrc=|\bhref=|\bonerror=|javascript:/i);
  for(const label of ["危険","外部","外部画像","ローカル","データ"])assert.ok(html.includes(label));
});
test("only a well-formed saved-audio interval becomes a play button, never an external href",()=>{
  const html=render("[00:00:01–00:00:02](audio:00:00:01-00:00:02) [別のリンク](audio:00:00:09-00:00:11) [不正](audio:00:00:02-00:00:01)");
  assert.equal((html.match(/<button /g)??[]).length,1);
  assert.ok(html.includes("この区間を再生: 00:00:01–00:00:02"));
  assert.ok(html.includes("<svg"));assert.doesNotMatch(html,/\bhref=/);
  assert.ok(html.includes("別のリンク"));assert.ok(html.includes("不正"));
});
test("missing audio and recording lock prevent interval play without changing Markdown text",()=>{
  const markdown="[区間](audio:00:00:01.000001-00:00:02.999999)";
  assert.doesNotMatch(render(markdown,null),/<button/);
  const html=render(markdown,10_000_000,true);assert.match(html,/<button[^>]*disabled=""/);
  assert.ok(html.includes("00:00:01.000001–00:00:02.999999"));assert.equal(markdown,"[区間](audio:00:00:01.000001-00:00:02.999999)");
});
