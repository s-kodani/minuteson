export type VocabularyEntry = { id: string; term: string; reading: string | null; note: string | null; enabled: boolean };
export type VocabularyStamp = { globalVersion: number; meetingVersion: number };

export function validVocabulary(entries: VocabularyEntry[]): boolean {
  const terms = new Set<string>();
  const ids = new Set<string>();
  return entries.length <= 500 && entries.every((e) => {
    if (!e.term || e.term.trim() !== e.term || [...e.term].length > 200 || /[\u0000-\u001f\u007f-\u009f]/u.test(e.term) || terms.has(e.term) || ids.has(e.id)) return false;
    if (e.reading !== null && (!e.reading.trim() || [...e.reading].length > 200 || /[\u0000-\u001f\u007f-\u009f]/u.test(e.reading))) return false;
    if (e.note !== null && (!e.note.trim() || [...e.note].length > 1000 || /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(e.note))) return false;
    terms.add(e.term); ids.add(e.id); return true;
  });
}

// Match the backend: enabled meeting terms first, exact spelling deduplicated.
// Disabled meeting terms do not suppress enabled common terms.
export function effectiveVocabulary(global: VocabularyEntry[], meeting: VocabularyEntry[]): VocabularyEntry[] {
  const seen = new Set<string>();
  return [...meeting, ...global].filter((entry) => {
    if (!entry.enabled || seen.has(entry.term)) return false;
    seen.add(entry.term);
    return true;
  });
}
