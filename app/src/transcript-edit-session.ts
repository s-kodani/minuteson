import type { Utterance } from "./transcript-state";

// Preserve canonical sequence and keyed DOM nodes when an edited/focused row
// no longer matches a filter. Counts remain those
// of the actual search, not the extra retained rows.
export function retainedTextRows(all: Utterance[], filtered: Utterance[], retained: string[]): { rows: Utterance[]; extra: number } {
  const visible = new Set(filtered.map(item => item.id));
  const ids = new Set([...visible, ...retained]);
  const rows = all.filter(item => ids.has(item.id));
  return { rows, extra: rows.filter(item => !visible.has(item.id)).length };
}
