import type { Block, Evidence, Inline, MinutesContent, MinutesItem, TextBlock } from "./minutes-state";

export interface InlineAddress { block: number; item?: number; row?: number; column?: number; header?: boolean }
export interface InlineSelection { address: InlineAddress; start: number; end: number }
export const textNode = (text: string): Inline => ({ type: "text", text, bold: false, italic: false });
export const plain = (nodes: Inline[]) => nodes.flatMap((n) => n.type === "text" ? [n.text] : []).join("");
export const dueLabel = (item: MinutesItem) => item.dueDate && item.dueText && item.dueDate !== item.dueText ? `${item.dueDate}（${item.dueText}）` : item.dueDate ?? item.dueText ?? "未確認";

// Verification is an explicit user annotation, not an engine or semantic edit.
// Never rewrite the anchor, source type, other links or generated original.
export function setEvidenceVerification(original: MinutesContent, id: string, verified: boolean): MinutesContent {
  const doc = structuredClone(original);
  const e = doc.evidence.find((e) => e.id === id);
  if (!e || !doc.items.some((i) => i.id === e.minutesItemId)) throw new Error("確認する根拠または文書項目がありません");
  e.verificationState = verified ? "user_verified" : "unverified";
  return doc;
}

export function compactInline(nodes: Inline[]): Inline[] {
  const result: Inline[] = [];
  for (const node of nodes) {
    const last = result.at(-1);
    if (node.type === "text" && last?.type === "text" && node.bold === last.bold && node.italic === last.italic) last.text += node.text;
    else if (node.type !== "text" || node.text) result.push({ ...node });
  }
  return result;
}
export function inlineAt(doc: MinutesContent, a: InlineAddress): Inline[] {
  const block = doc.blocks[a.block];
  if (!block) throw new Error("編集対象の文書要素がありません");
  if (block.type === "heading" || block.type === "paragraph") return block.content;
  if (block.type === "list") return block.items[a.item ?? 0].content;
  return a.header ? block.headers[a.column ?? 0] : block.rows[a.row ?? 0].cells[a.column ?? 0];
}
function ownerAt(doc: MinutesContent, a: InlineAddress): string | null {
  const b = doc.blocks[a.block];
  if (b.type === "heading" || b.type === "paragraph") return b.itemId ?? null;
  if (b.type === "list") return b.items[a.item ?? 0].itemId;
  return a.header ? null : b.rows[a.row ?? 0].itemId;
}
function putInline(doc: MinutesContent, a: InlineAddress, nodes: Inline[]) {
  const b = doc.blocks[a.block];
  if (b.type === "heading" || b.type === "paragraph") b.content = nodes;
  else if (b.type === "list") b.items[a.item ?? 0].content = nodes;
  else if (a.header) b.headers[a.column ?? 0] = nodes;
  else b.rows[a.row ?? 0].cells[a.column ?? 0] = nodes;
}
function manual(doc: MinutesContent, id: string, resetVerification: boolean) {
  const item = doc.items.find((i) => i.id === id);
  if (!item) throw new Error("文書項目がありません");
  item.sourceType = "manual";
  for (const e of doc.evidence.filter((e) => e.minutesItemId === id)) {
    e.sourceType = "manual";
    if (resetVerification) e.verificationState = "unverified";
  }
}
export function reconcileDocument(doc: MinutesContent): MinutesContent {
  const owners = new Set<string>();
  const chips = new Set<string>();
  const visit = (nodes: Inline[]) => { for (const n of nodes) if (n.type === "evidence") chips.add(n.evidenceLinkId); };
  for (const b of doc.blocks) {
    if (b.type === "heading" || b.type === "paragraph") { if (b.itemId) owners.add(b.itemId); visit(b.content); }
    else if (b.type === "list") for (const row of b.items) { if (row.itemId) owners.add(row.itemId); visit(row.content); }
    else { for (const cell of b.headers) visit(cell); for (const row of b.rows) { if (row.itemId) owners.add(row.itemId); for (const cell of row.cells) visit(cell); } }
  }
  doc.items = doc.items.filter((i) => owners.has(i.id));
  const removed = doc.evidence.filter((e) => owners.has(e.minutesItemId) && !chips.has(e.id));
  doc.evidence = doc.evidence.filter((e) => owners.has(e.minutesItemId) && chips.has(e.id));
  for (const e of removed) manual(doc, e.minutesItemId, true);
  return doc;
}
export function editInline(original: MinutesContent, address: InlineAddress, value: Inline[], undoEvidence: readonly Evidence[] = []): MinutesContent {
  const doc = structuredClone(original);
  const before = inlineAt(doc, address);
  const nodes = compactInline(value);
  const owner = ownerAt(doc, address);
  const allowed = new Set(before.flatMap((n) => n.type === "evidence" ? [n.evidenceLinkId] : []));
  let recovered = false;
  // Only the editor's same-epoch native Undo/Redo may supply previously removed
  // fixed references. Ordinary typing/paste still cannot introduce a link.
  for (const node of nodes) if (node.type === "evidence" && !allowed.has(node.evidenceLinkId)) {
    const restored = undoEvidence.find(e => e.id === node.evidenceLinkId && e.minutesItemId === owner && node.minutesItemId === owner);
    if (restored && !doc.evidence.some(e => e.id === restored.id)) {
      doc.evidence.push({ ...restored, sourceType: "manual", verificationState: "unverified" });
      allowed.add(restored.id);
      recovered = true;
    }
  }
  if (recovered) {
    const order = new Map(undoEvidence.map((e, index) => [e.id, index]));
    doc.evidence.sort((a, b) => (order.get(a.id) ?? Infinity) - (order.get(b.id) ?? Infinity));
  }
  if (nodes.some((n) => n.type === "evidence" && (!allowed.has(n.evidenceLinkId) || n.minutesItemId !== owner))) throw new Error("別の項目の根拠は貼り付けできません");
  const b = doc.blocks[address.block];
  if (b.type === "table" && owner && address.column === 3 && plain(nodes).trim()) throw new Error("ToDoの根拠欄には固定参照だけを保持します");
  putInline(doc, address, nodes);
  if (owner && plain(before) !== plain(nodes)) {
    const item = doc.items.find((i) => i.id === owner)!;
    const body = plain(nodes);
    if (b.type !== "table" || address.column === 0) item.body = body;
    else if (address.column === 1) { item.assigneeName = !body.trim() || body === "未確認" ? null : body; item.assigneeSpeakerId = null; }
    else if (address.column === 2) {
      item.dueDate = /^\d{4}-\d{2}-\d{2}$/.test(body) ? body : null;
      item.dueText = !body.trim() || body === "未確認" || item.dueDate ? null : body;
    }
    manual(doc, owner, true);
    if (b.type === "table" && address.column === 1 && !item.assigneeName) putInline(doc, address, [textNode("未確認")]);
    if (b.type === "table" && address.column === 2 && !item.dueDate && !item.dueText) putInline(doc, address, [textNode("未確認")]);
  }
  return reconcileDocument(doc);
}
// Selection offsets count UTF-16 text units and each atomic evidence chip as
// one unit. Formatting never rewrites, duplicates or detaches a fixed link.
export function formatInline(nodes: Inline[], start: number, end: number, mark: "bold" | "italic"): Inline[] {
  if (end <= start) return structuredClone(nodes);
  let offset = 0;
  const covered = nodes.filter((n) => { const length = n.type === "text" ? n.text.length : 1; const selected = offset < end && offset + length > start; offset += length; return selected && n.type === "text"; });
  const enabled = !covered.every((n) => n.type === "text" && n[mark]);
  offset = 0;
  const result: Inline[] = [];
  for (const n of nodes) {
    const length = n.type === "text" ? n.text.length : 1;
    if (n.type === "evidence") result.push({ ...n });
    else {
      const boundary = (value: number, forward: boolean) => {
        const at = Math.max(0, Math.min(length, value));
        const previous = n.text.charCodeAt(at - 1); const next = n.text.charCodeAt(at);
        return previous >= 0xd800 && previous <= 0xdbff && next >= 0xdc00 && next <= 0xdfff ? at + (forward ? 1 : -1) : at;
      };
      const from = boundary(start - offset, false);
      const to = Math.max(from, boundary(end - offset, true));
      if (from) result.push({ ...n, text: n.text.slice(0, from) });
      if (to > from) result.push({ ...n, text: n.text.slice(from, to), [mark]: enabled });
      if (to < length) result.push({ ...n, text: n.text.slice(to) });
    }
    offset += length;
  }
  return compactInline(result);
}
export function convertBlock(original: MinutesContent, a: InlineAddress, type: "paragraph" | "heading" | "list", level = 2, ordered = false): MinutesContent {
  const doc = structuredClone(original); const b = doc.blocks[a.block];
  if (b.type === "table") throw new Error("表は行・列として編集してください");
  if (type === "list" && b.type === "list") { b.ordered = ordered; return doc; }
  const row: TextBlock = b.type === "list" ? b.items[a.item ?? 0] : { itemId: b.itemId ?? null, content: b.content };
  const replacement: Block = type === "heading" ? { type, level, ...(row.itemId ? { itemId: row.itemId } : {}), content: row.content }
    : type === "paragraph" ? { type, ...row } : { type, ordered, items: [row] };
  const blocks: Block[] = [];
  if (b.type === "list" && (a.item ?? 0) > 0) blocks.push({ ...b, items: b.items.slice(0, a.item) });
  blocks.push(replacement);
  if (b.type === "list" && (a.item ?? 0) + 1 < b.items.length) blocks.push({ ...b, items: b.items.slice((a.item ?? 0) + 1) });
  doc.blocks.splice(a.block, 1, ...blocks);
  return doc;
}
export function insertBlock(original: MinutesContent, after: number, type: "paragraph" | "heading" | "list" | "table"): MinutesContent {
  const doc = structuredClone(original);
  const value: Block = type === "heading" ? { type, level: 2, content: [textNode("新しい見出し")] }
    : type === "paragraph" ? { type, itemId: null, content: [textNode("")] }
    : type === "list" ? { type, ordered: false, items: [{ itemId: null, content: [textNode("新しい項目")] }] }
    : { type, headers: [[textNode("列1")], [textNode("列2")]], rows: [{ itemId: null, cells: [[textNode("")], [textNode("")]] }] };
  doc.blocks.splice(Math.min(doc.blocks.length, Math.max(0, after + 1)), 0, value);
  return doc;
}
export function removeBlock(original: MinutesContent, index: number): MinutesContent {
  const doc = structuredClone(original); doc.blocks.splice(index, 1); return reconcileDocument(doc);
}
export function changeList(original: MinutesContent, a: InlineAddress, remove: boolean): MinutesContent {
  const doc = structuredClone(original); const b = doc.blocks[a.block];
  if (b.type !== "list") throw new Error("箇条書きを選択してください");
  if (remove) b.items.splice(a.item ?? 0, 1);
  else b.items.splice((a.item ?? b.items.length - 1) + 1, 0, { itemId: null, content: [textNode("新しい項目")] });
  return reconcileDocument(doc);
}
export function changeTable(original: MinutesContent, a: InlineAddress, action: "addRow" | "removeRow" | "addColumn" | "removeColumn"): MinutesContent {
  const doc = structuredClone(original); const b = doc.blocks[a.block];
  if (b.type !== "table") throw new Error("表を選択してください");
  const todo = b.rows.some((r) => r.itemId) || (b.headers.length === 4 && plain(b.headers[1]) === "担当者" && plain(b.headers[2]) === "期限");
  if (action === "addRow") {
    const item = todo ? { id: crypto.randomUUID(), kind: "todo" as const, body: "新しいToDo", assigneeName: null, assigneeSpeakerId: null, dueDate: null, dueText: null, sourceType: "manual" as const } : null;
    if (item) doc.items.push(item);
    b.rows.splice((a.row ?? b.rows.length - 1) + 1, 0, { itemId: item?.id ?? null, cells: item ? [[textNode(item.body)], [textNode("未確認")], [textNode("未確認")], []] : b.headers.map(() => []) });
  } else if (action === "removeRow") b.rows.splice(a.row ?? 0, 1);
  else {
    if (todo) throw new Error("ToDo表は内容・担当者・期限・根拠の4列を保持します");
    if (action === "addColumn") { if (b.headers.length >= 32) throw new Error("表は32列までです"); b.headers.push([textNode("新しい列")]); for (const r of b.rows) r.cells.push([]); }
    else { if (b.headers.length <= 1) throw new Error("表には1列以上必要です"); const c = a.column ?? b.headers.length - 1; b.headers.splice(c, 1); for (const r of b.rows) r.cells.splice(c, 1); }
  }
  return reconcileDocument(doc);
}
