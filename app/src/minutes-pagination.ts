import type { Block } from "./minutes-state";
import type { InlineAddress } from "./minutes-edit";

// Presentation only: never slice the canonical document passed to save/export.
// A long list or table consumes one unit per body row, not one per block.
export const MINUTES_PAGE_SIZE = 50;
export interface DocumentFragment { blockIndex: number; rowStart: number; rowEnd: number }
export interface DocumentPage { page: number; pages: number; units: number; fragments: DocumentFragment[] }
export interface DocumentOutlineEntry { blockIndex: number; level: number; label: string; page: number }
const rows = (block: Block) => block.type === "list" ? block.items.length : block.type === "table" ? block.rows.length : 1;
const count = (block: Block) => Math.max(1, rows(block));
const clamp = (value: number, length: number) => Math.max(0, Math.min(Number.isFinite(value) ? Math.trunc(value) : 0, Math.max(0, length - 1)));

// Derive navigation from the full canonical blocks, not just the current page.
// Evidence chips carry no invented heading text; labels remain plain strings.
export function documentOutline(blocks: Block[]): DocumentOutlineEntry[] {
  let offset = 0;
  const entries: DocumentOutlineEntry[] = [];
  blocks.forEach((block, blockIndex) => {
    if (block.type === "heading") entries.push({ blockIndex, level: block.level,
      label: block.content.map(node => node.type === "text" ? node.text : "").join("").replace(/\s+/gu," ").trim() || `見出し${blockIndex + 1}（本文なし）`,
      page: Math.floor(offset / MINUTES_PAGE_SIZE) });
    offset += count(block);
  });
  return entries;
}

export function documentPage(blocks: Block[], requested: number): DocumentPage {
  const units = blocks.reduce((sum, block) => sum + count(block), 0);
  const pages = Math.max(1, Math.ceil(units / MINUTES_PAGE_SIZE));
  const page = clamp(requested, pages);
  const start = page * MINUTES_PAGE_SIZE, end = start + MINUTES_PAGE_SIZE;
  const fragments: DocumentFragment[] = [];
  let offset = 0;
  for (let blockIndex = 0; blockIndex < blocks.length; blockIndex++) {
    const block = blocks[blockIndex], size = count(block);
    if (offset >= end) break;
    if (offset + size > start) fragments.push({ blockIndex, rowStart: Math.max(0, start - offset), rowEnd: Math.min(rows(block), end - offset) });
    offset += size;
  }
  return { page, pages, units, fragments };
}

// Resolve structure changes/deletions back to a real field in the full document.
export function documentAddress(blocks: Block[], requested: InlineAddress): InlineAddress | null {
  if (!blocks.length) return null;
  const block = clamp(requested.block, blocks.length), value = blocks[block];
  if (value.type === "heading" || value.type === "paragraph") return { block };
  if (value.type === "list") return value.items.length ? { block, item: clamp(requested.item ?? 0, value.items.length) } : null;
  const column = clamp(requested.column ?? 0, value.headers.length);
  return requested.header || !value.rows.length ? { block, header: true, column } : { block, row: clamp(requested.row ?? 0, value.rows.length), column };
}

export function documentPageForAddress(blocks: Block[], requested: InlineAddress): number {
  if (!blocks.length) return 0;
  const block = clamp(requested.block, blocks.length), value = blocks[block];
  const preceding = blocks.slice(0, block).reduce((sum, b) => sum + count(b), 0);
  const row = value.type === "list" ? clamp(requested.item ?? 0, value.items.length)
    : value.type === "table" && !requested.header ? clamp(requested.row ?? 0, value.rows.length) : 0;
  return Math.floor((preceding + row) / MINUTES_PAGE_SIZE);
}

export function firstDocumentAddress(blocks: Block[], page: number): InlineAddress | null {
  for (const fragment of documentPage(blocks, page).fragments) {
    const address = documentAddress(blocks, { block: fragment.blockIndex, item: fragment.rowStart, row: fragment.rowStart, column: 0 });
    if (address) return address;
  }
  return null;
}

export const inlineAddressKey = (a: InlineAddress) => `${a.block}:${a.item ?? ""}:${a.row ?? ""}:${a.column ?? ""}:${a.header ? "header" : ""}`;
