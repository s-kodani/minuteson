import test from "node:test";
import assert from "node:assert/strict";
import { documentAddress, documentOutline, documentPage, documentPageForAddress, firstDocumentAddress, inlineAddressKey, MINUTES_PAGE_SIZE } from "../src/minutes-pagination.ts";
import { changeList, changeTable, convertBlock, editInline, inlineAt, plain, textNode } from "../src/minutes-edit.ts";
import type { Block, MinutesContent } from "../src/minutes-state.ts";

const list = (length: number): Block => ({ type: "list", ordered: true, items: Array.from({ length }, (_, i) => ({ itemId: null, content: [textNode(`項目${i + 1}`)] })) });
const table = (length: number): Block => ({ type: "table", headers: [[textNode("列1")], [textNode("列2")]], rows: Array.from({ length }, (_, i) => ({ itemId: null, cells: [[textNode(`行${i + 1}`)], [textNode("メモ")]] })) });
const paragraph = (): Block => ({ type: "paragraph", itemId: null, content: [textNode("本文")] });

test("outline covers canonical headings across long list/table pages without changing document nodes", () => {
  const blocks: Block[] = [{ type:"heading",level:1,content:[textNode(" 会議\n概要 ")] },list(121),
    { type:"heading",level:2,content:[textNode("決定事項")] },table(125),
    { type:"heading",level:6,content:[textNode("決定事項")] }];
  const before=structuredClone(blocks);
  assert.deepEqual(documentOutline(blocks),[
    { blockIndex:0,level:1,label:"会議 概要",page:0 },
    { blockIndex:2,level:2,label:"決定事項",page:2 },
    { blockIndex:4,level:6,label:"決定事項",page:4 },
  ]);
  for(const entry of documentOutline(blocks)) assert.equal(entry.page,documentPageForAddress(blocks,{block:entry.blockIndex}));
  assert.deepEqual(blocks,before);
});

test("outline never invents words from evidence or interprets heading text as HTML", () => {
  const blocks: Block[]=[{type:"heading",level:2,content:[{type:"evidence",minutesItemId:"item",evidenceLinkId:"e"}]},
    {type:"heading",level:3,content:[textNode("<img src=x onerror=alert(1)>"),textNode("\n確認")]},list(0),table(0)];
  assert.deepEqual(documentOutline(blocks),[
    {blockIndex:0,level:2,label:"見出し1（本文なし）",page:0},
    {blockIndex:1,level:3,label:"<img src=x onerror=alert(1)> 確認",page:0},
  ]);
  assert.deepEqual(documentOutline([paragraph(),list(1),table(1)]),[]);
  assert.deepEqual(documentOutline([]),[]);
});

test("outline respects empty-block units and headings exactly at a pagination boundary", () => {
  const blocks: Block[]=[list(0),table(0),list(48),{type:"heading",level:4,content:[textNode("境界の見出し")]}];
  assert.deepEqual(documentOutline(blocks),[{blockIndex:3,level:4,label:"境界の見出し",page:1}]);
});

test("empty and boundary-sized documents consume body units, preserve empty blocks and clamp page requests", () => {
  assert.deepEqual(documentPage([], 9), { page: 0, pages: 1, units: 0, fragments: [] });
  for (const length of [0, 1, 49, 50, 51, 100, 101]) {
    for (const block of [list(length), table(length)]) {
      const pages = Math.max(1, Math.ceil(length / MINUTES_PAGE_SIZE));
      assert.equal(documentPage([block], 0).pages, pages);
      assert.equal(documentPage([block], -1).page, 0);
      assert.equal(documentPage([block], NaN).page, 0);
      assert.equal(documentPage([block], 999).page, pages - 1);
      if (!length) assert.deepEqual(documentPage([block], 0).fragments, [{ blockIndex: 0, rowStart: 0, rowEnd: 0 }]);
    }
  }
  assert.equal(documentPage(Array.from({length:51},paragraph), 0).fragments.length, 50);
});

test("mixed long lists and tables cover every full-document row exactly once with at most 50 body units", () => {
  const blocks = [paragraph(), list(121), table(125), list(0), table(0), paragraph()];
  const before = structuredClone(blocks), seen: string[] = [];
  const expected = blocks.flatMap((b, i) => b.type === "list" ? Array.from({length:Math.max(1,b.items.length)},(_,r)=>`${i}:${r}`) : b.type === "table" ? Array.from({length:Math.max(1,b.rows.length)},(_,r)=>`${i}:${r}`) : [`${i}:0`]);
  const pages = documentPage(blocks, 0).pages;
  for (let page = 0; page < pages; page++) {
    const plan = documentPage(blocks, page);
    let units = 0;
    for (const f of plan.fragments) {
      units += Math.max(1, f.rowEnd - f.rowStart);
      for (let r = f.rowStart; r < Math.max(f.rowStart + 1, f.rowEnd); r++) seen.push(`${f.blockIndex}:${r}`);
    }
    assert.ok(units <= MINUTES_PAGE_SIZE);
    if (page < pages - 1) assert.equal(units, MINUTES_PAGE_SIZE);
  }
  assert.deepEqual(seen, expected); assert.equal(new Set(seen).size, seen.length);
  assert.deepEqual(blocks, before);
});

test("page fragments retain global edit addresses including last rows and repeated table headers", () => {
  const blocks = [paragraph(), list(121), table(125)];
  assert.equal(documentPageForAddress(blocks, {block:1,item:120}), 2);
  assert.equal(documentPageForAddress(blocks, {block:2,row:124,column:1}), 4);
  assert.equal(documentPageForAddress(blocks, {block:2,header:true,column:1}), 2);
  assert.deepEqual(firstDocumentAddress(blocks, 1), {block:1,item:49});
  assert.deepEqual(firstDocumentAddress(blocks, 3), {block:2,row:28,column:0});
  assert.deepEqual(documentAddress(blocks, {block:9,row:999,column:99}), {block:2,row:124,column:1});
  assert.equal(inlineAddressKey({block:2,row:124,column:1}), inlineAddressKey({column:1,block:2,row:124}));
  assert.equal(firstDocumentAddress([list(0)], 0), null);
  assert.deepEqual(firstDocumentAddress([list(0),table(0)], 0), {block:1,header:true,column:0});
});

test("late-page edits change only the addressed canonical rows and preserve all undisplayed rows", () => {
  const doc: MinutesContent = {schemaVersion:1,blocks:[paragraph(),list(121),table(125)],items:[],evidence:[]};
  const listAddress = documentAddress(doc.blocks,{block:1,item:120})!;
  const tableAddress = documentAddress(doc.blocks,{block:2,row:124,column:1})!;
  const edited = editInline(editInline(doc,listAddress,[textNode("末尾項目を修正")]),tableAddress,[textNode("末尾セルを修正")]);
  assert.equal(plain(inlineAt(edited,listAddress)),"末尾項目を修正");
  assert.equal(plain(inlineAt(edited,tableAddress)),"末尾セルを修正");
  const expected = structuredClone(doc);
  if(expected.blocks[1].type!=="list"||expected.blocks[2].type!=="table")throw Error("fixture");
  expected.blocks[1].items[120].content=[textNode("末尾項目を修正")]; expected.blocks[2].rows[124].cells[1]=[textNode("末尾セルを修正")];
  assert.deepEqual(edited,expected); assert.equal(plain(inlineAt(doc,listAddress)),"項目121");
});

test("adding, removing and converting elements at page boundaries resolves a real global focus target", () => {
  const doc: MinutesContent = {schemaVersion:1,blocks:[list(100),table(100)],items:[],evidence:[]};
  const added = changeList(doc,{block:0,item:49},false);
  const focus = documentAddress(added.blocks,{block:0,item:50})!;
  assert.equal(plain(inlineAt(added,focus)),"新しい項目"); assert.equal(documentPageForAddress(added.blocks,focus),1);
  assert.deepEqual(changeList(added,focus,true),doc);
  const rowAdded = changeTable(doc,{block:1,row:49},"addRow");
  assert.equal(documentPageForAddress(rowAdded.blocks,{block:1,row:50}),3);
  assert.deepEqual(changeTable(rowAdded,{block:1,row:50},"removeRow"),doc);
  const removed = changeTable(doc,{block:1,row:99},"removeRow");
  assert.deepEqual(documentAddress(removed.blocks,{block:1,row:99,column:1}),{block:1,row:98,column:1});
  const converted = convertBlock(doc,{block:0,item:99},"heading",3);
  const target = documentAddress(converted.blocks,{block:1})!;
  assert.equal(plain(inlineAt(converted,target)),"項目100"); assert.equal(documentPageForAddress(converted.blocks,target),1);
});
