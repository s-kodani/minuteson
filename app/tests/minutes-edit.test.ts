import test from "node:test";
import assert from "node:assert/strict";
import { changeList, changeTable, compactInline, convertBlock, editInline, formatInline, inlineAt, insertBlock, plain, removeBlock, setEvidenceVerification, textNode } from "../src/minutes-edit.ts";
import type { Inline, MinutesContent } from "../src/minutes-state.ts";

const chip: Inline = { type: "evidence", minutesItemId: "decision", evidenceLinkId: "link" };
function document(): MinutesContent {
  return { schemaVersion: 1, blocks: [
    { type: "heading", level: 1, content: [textNode("会議")] },
    { type: "list", ordered: false, items: [{ itemId: "decision", content: [textNode("点検日を確認"), chip] }, { itemId: null, content: [textNode("自由入力")] }] },
    { type: "table", headers: [[textNode("内容")], [textNode("担当者")], [textNode("期限")], [textNode("根拠")]], rows: [{ itemId: "todo", cells: [[textNode("資料を送る")], [textNode("田中")], [textNode("2026-10-09（来週金曜）")], [{ type: "evidence", minutesItemId: "todo", evidenceLinkId: "todo-link" }]] }] },
  ], items: [
    { id: "decision", kind: "decision", body: "点検日を確認", assigneeName: null, assigneeSpeakerId: null, dueDate: null, dueText: null, sourceType: "engine" },
    { id: "todo", kind: "todo", body: "資料を送る", assigneeName: "田中", assigneeSpeakerId: "speaker", dueDate: "2026-10-09", dueText: "来週金曜", sourceType: "engine" },
  ], evidence: [
    { id: "link", minutesItemId: "decision", analysisRevisionId: "fixed", utteranceId: "u", quoteText: "点検日を確認", startUs: 1, endUs: 2, sourceType: "engine", verificationState: "user_verified" },
    { id: "todo-link", minutesItemId: "todo", analysisRevisionId: "fixed", utteranceId: "u", quoteText: "資料を送る", startUs: 1, endUs: 2, sourceType: "engine", verificationState: "unverified" },
  ] };
}
test("explicit verification changes only one annotation, retains frozen anchors and source, and can be cleared", () => {
  const original = document();
  const checked = setEvidenceVerification(original, "todo-link", true);
  assert.deepEqual(checked.evidence[1], {...original.evidence[1], verificationState:"user_verified"});
  assert.deepEqual(checked.blocks,original.blocks); assert.deepEqual(checked.items,original.items); assert.deepEqual(checked.evidence[0],original.evidence[0]);
  assert.equal(original.evidence[1].verificationState,"unverified");
  assert.deepEqual(setEvidenceVerification(checked,"todo-link",false),original);
  assert.throws(()=>setEvidenceVerification(original,"missing",true));
  assert.throws(()=>setEvidenceVerification({...original,items:[]},"link",true));
  const edited=editInline(checked,{block:2,row:0,column:1},[textNode("別の担当者")]);
  assert.equal(edited.evidence[1].verificationState,"unverified"); assert.equal(edited.evidence[1].sourceType,"manual");
  assert.equal(edited.evidence[0].verificationState,"user_verified");
});
test("semantic rich-text changes update the item and mark its fixed evidence manual without rewriting the anchor", () => {
  const original = document(); const address = { block: 1, item: 0 };
  const edited = editInline(original, address, [textNode("点検日を再確認する"), chip]);
  assert.equal(edited.items[0].body, "点検日を再確認する"); assert.equal(edited.items[0].sourceType, "manual");
  assert.deepEqual(edited.evidence[0], { ...original.evidence[0], sourceType: "manual", verificationState: "unverified" });
  assert.equal(original.items[0].body, "点検日を確認");
  assert.throws(() => editInline(original, address, [{ ...chip, evidenceLinkId: "other" }]));
  assert.throws(() => editInline(original, address, [{ ...chip, minutesItemId: "todo" }]));
});
test("selection formatting keeps every atomic chip and does not change item semantics or verification", () => {
  const original = document(); const address = { block: 1, item: 0 };
  const result = formatInline(inlineAt(original, address), 0, 3, "bold");
  assert.deepEqual(result, [{ type: "text", text: "点検日", bold: true, italic: false }, textNode("を確認"), chip]);
  const edited = editInline(original, address, result);
  assert.deepEqual(edited.items, original.items); assert.deepEqual(edited.evidence, original.evidence);
  assert.deepEqual(formatInline(result, 0, 3, "bold"), inlineAt(original, address));
  assert.equal(plain(formatInline([textNode("A😀B"), chip], 1, 3, "italic")), "A😀B");
  assert.deepEqual(formatInline([textNode("A😀B"), chip], 2, 3, "italic"), [textNode("A"), { ...textNode("😀"), italic: true }, textNode("B"), chip]);
  assert.deepEqual(formatInline([chip], 0, 1, "bold"), [chip]);
  assert.deepEqual(compactInline([textNode("a"), textNode(""), textNode("b")]), [textNode("ab")]);
});
test("paragraph/list/heading conversions retain owners and references and split multi-item lists in document order", () => {
  const original = document(); const converted = convertBlock(original, { block: 1, item: 0 }, "heading", 3);
  assert.deepEqual(converted.blocks[1], { type: "heading", level: 3, itemId: "decision", content: [textNode("点検日を確認"), chip] });
  assert.deepEqual(converted.items, original.items); assert.deepEqual(converted.evidence, original.evidence);
  assert.deepEqual(converted.blocks[2], { type: "list", ordered: false, items: [{ itemId: null, content: [textNode("自由入力")] }] });
  assert.equal(convertBlock(converted, { block: 1 }, "paragraph").blocks[1].type, "paragraph");
  assert.equal((convertBlock(original, { block: 1 }, "list", 2, true).blocks[1] as { ordered: boolean }).ordered, true);
  assert.throws(() => convertBlock(original, { block: 2, row: 0 }, "heading"));
});
test("ToDo metadata edits are explicit, no dates or speakers are guessed, formatting alone retains the original expression", () => {
  const original = document();
  const formatted = editInline(original, { block: 2, row: 0, column: 2 }, formatInline(inlineAt(original, { block: 2, row: 0, column: 2 }), 0, 10, "bold"));
  assert.deepEqual(formatted.items, original.items);
  const assignee = editInline(original, { block: 2, row: 0, column: 1 }, [textNode("社外担当")]);
  assert.equal(assignee.items[1].assigneeName, "社外担当"); assert.equal(assignee.items[1].assigneeSpeakerId, null);
  assert.equal(assignee.evidence[1].sourceType, "manual");
  const relative = editInline(original, { block: 2, row: 0, column: 2 }, [textNode("来週中")]);
  assert.equal(relative.items[1].dueDate, null); assert.equal(relative.items[1].dueText, "来週中");
  const literal = editInline(relative, { block: 2, row: 0, column: 2 }, [textNode("2026-11-01")]);
  assert.equal(literal.items[1].dueDate, "2026-11-01"); assert.equal(literal.items[1].dueText, null);
  const unknown = editInline(original, { block: 2, row: 0, column: 1 }, []);
  assert.equal(unknown.items[1].assigneeName, null); assert.equal(plain(inlineAt(unknown, { block: 2, row: 0, column: 1 })), "未確認");
  assert.throws(() => editInline(original, { block: 2, row: 0, column: 3 }, [textNode("架空の根拠")]));
});
test("deleting chips, items and blocks reconciles all annotations while leaving the generated input immutable", () => {
  const original = document(); const removedChip = editInline(original, { block: 1, item: 0 }, [textNode("点検日を確認")]);
  assert.equal(removedChip.items[0].sourceType, "manual"); assert.equal(removedChip.evidence.length, 1);
  const removedItem = changeList(original, { block: 1, item: 0 }, true);
  assert.deepEqual(removedItem.items.map(i => i.id), ["todo"]); assert.deepEqual(removedItem.evidence.map(e => e.id), ["todo-link"]);
  const removedBlock = removeBlock(original, 2);
  assert.deepEqual(removedBlock.items.map(i => i.id), ["decision"]); assert.deepEqual(removedBlock.evidence.map(e => e.id), ["link"]);
  assert.equal(original.evidence.length, 2);
});
test("native text Undo restores only the same owner's frozen reference from its editing epoch", () => {
  const original = document(), address = { block: 1, item: 0 };
  const removed = editInline(original, address, []);
  assert.throws(() => editInline(removed, address, inlineAt(original, address)), /別の項目/);
  const restored = editInline(removed, address, inlineAt(original, address), original.evidence);
  assert.deepEqual(inlineAt(restored, address), inlineAt(original, address));
  assert.deepEqual(restored.evidence.find(e => e.id === "link"), { ...original.evidence[0], sourceType: "manual", verificationState: "unverified" });
  assert.deepEqual(restored.evidence.find(e => e.id === "todo-link"), original.evidence[1]);
  assert.deepEqual(restored.evidence.map(e => e.id), original.evidence.map(e => e.id));
  assert.equal(restored.items[0].body, original.items[0].body);
  assert.throws(() => editInline(removed, address, [{ ...chip, minutesItemId: "todo", evidenceLinkId: "todo-link" }], original.evidence), /別の項目/);
  assert.throws(() => editInline(removed, address, [chip], []), /別の項目/);
  assert.throws(() => editInline(removed, address, [chip], [{ ...original.evidence[0], minutesItemId: "todo" }]), /別の項目/);
  assert.equal(original.evidence.length, 2);
  assert.equal(removed.evidence.length, 1);
  const redone = editInline(restored, address, []);
  assert.equal(redone.evidence.length, 1);
  assert.deepEqual(inlineAt(editInline(redone, address, inlineAt(original, address), restored.evidence), address), inlineAt(original, address));
});
test("table row insertion adds a manual ungrounded ToDo with unknown metadata, generic table columns remain rectangular", () => {
  const original = document(); const added = changeTable(original, { block: 2, row: 0 }, "addRow");
  assert.equal(added.items.length, 3); assert.equal(added.items[2].sourceType, "manual"); assert.equal(added.items[2].assigneeName, null); assert.equal(added.items[2].dueDate, null); assert.equal(added.evidence.length, 2);
  assert.deepEqual(changeTable(added, { block: 2, row: 1 }, "removeRow"), original);
  assert.throws(() => changeTable(original, { block: 2 }, "addColumn"));
  const table = insertBlock(original, 2, "table"); const columns = changeTable(table, { block: 3 }, "addColumn");
  const b = columns.blocks[3]; assert.equal(b.type, "table"); if (b.type !== "table") throw Error("table");
  assert.equal(b.headers.length, 3); assert.ok(b.rows.every(r => r.cells.length === 3));
  assert.equal((changeTable(columns, { block: 3, column: 1 }, "removeColumn").blocks[3] as { headers: Inline[][] }).headers.length, 2);
});
