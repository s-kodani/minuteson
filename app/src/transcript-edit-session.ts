import type { DraftPayload, Utterance } from "./transcript-state";

export interface TextEditBaseline {
  utteranceId: string;
  text: string;
  speakerIds: string[];
  hadOverride: boolean;
}

export interface TextEditSession extends TextEditBaseline { value: string }

// Only a confirmed field (or the navigation/close save barrier) enters the
// durable queue. Other open text buffers stay independent of this operation.
export function stageTextEdit(payload: DraftPayload, session: TextEditSession): DraftPayload {
  const entry = payload.utterances.find(item => item.utterance_id === session.utteranceId);
  if (session.value === (entry?.effective_text ?? session.text)) return payload;
  return { ...payload, utterances: [...payload.utterances.filter(item => item.utterance_id !== session.utteranceId), {
    utterance_id: session.utteranceId, effective_text: session.value, speaker_ids: entry?.speaker_ids ?? [...session.speakerIds],
  }] };
}

export function textBuffersDirty(sessions: Record<string, TextEditSession>, utterances: Utterance[]): boolean {
  const values = new Map(utterances.map(item => [item.id, item.effectiveText]));
  return Object.values(sessions).some(session => values.has(session.utteranceId) && session.value !== values.get(session.utteranceId));
}

// Preserve canonical sequence and keyed DOM nodes when an edited/focused row
// no longer matches a filter. Counts remain those
// of the actual search, not the extra retained rows.
export function retainedTextRows(all: Utterance[], filtered: Utterance[], retained: string[]): { rows: Utterance[]; extra: number } {
  const visible = new Set(filtered.map(item => item.id));
  const ids = new Set([...visible, ...retained]);
  const rows = all.filter(item => ids.has(item.id));
  return { rows, extra: rows.filter(item => !visible.has(item.id)).length };
}

export function textEditBaseline(payload: DraftPayload, utterance: Utterance): TextEditBaseline {
  const entry = payload.utterances.find(item => item.utterance_id === utterance.id);
  return { utteranceId: utterance.id, text: entry?.effective_text ?? utterance.effectiveText,
    speakerIds: [...(entry?.speaker_ids ?? utterance.speakerIds)], hadOverride: !!entry };
}

// Cancel only this edit session's text, not unrelated names, assignments or
// structural edits. If a save barrier has already persisted it, the queue writes
// the restoration as a new durable edit; acknowledged history is not erased.
export function cancelTextEdit(payload: DraftPayload, baseline: TextEditBaseline): DraftPayload {
  const entry = payload.utterances.find(item => item.utterance_id === baseline.utteranceId);
  if (!entry) return payload;
  const sameSpeakers = JSON.stringify([...entry.speaker_ids].sort()) === JSON.stringify([...baseline.speakerIds].sort());
  return { ...payload, utterances: payload.utterances.flatMap(item => item !== entry ? [item]
    : !baseline.hadOverride && sameSpeakers ? [] : [{ ...entry, effective_text: baseline.text }]) };
}
