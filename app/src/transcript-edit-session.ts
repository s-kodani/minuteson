import type { DraftPayload, Utterance } from "./transcript-state";

export interface TextEditBaseline {
  utteranceId: string;
  text: string;
  speakerIds: string[];
  hadOverride: boolean;
}

export function textEditBaseline(payload: DraftPayload, utterance: Utterance): TextEditBaseline {
  const entry = payload.utterances.find(item => item.utterance_id === utterance.id);
  return { utteranceId: utterance.id, text: entry?.effective_text ?? utterance.effectiveText,
    speakerIds: [...(entry?.speaker_ids ?? utterance.speakerIds)], hadOverride: !!entry };
}

// Cancel only this focus session's text, not unrelated names, assignments or
// structural edits. If autosave has already persisted it, the save queue writes
// the restoration as a new durable edit; acknowledged history is not erased.
export function cancelTextEdit(payload: DraftPayload, baseline: TextEditBaseline): DraftPayload {
  const entry = payload.utterances.find(item => item.utterance_id === baseline.utteranceId);
  if (!entry) return payload;
  const sameSpeakers = JSON.stringify([...entry.speaker_ids].sort()) === JSON.stringify([...baseline.speakerIds].sort());
  return { ...payload, utterances: payload.utterances.flatMap(item => item !== entry ? [item]
    : !baseline.hadOverride && sameSpeakers ? [] : [{ ...entry, effective_text: baseline.text }]) };
}
