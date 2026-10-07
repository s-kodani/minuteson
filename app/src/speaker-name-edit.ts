import type { DraftPayload, Speaker } from "./transcript-state.ts";

export interface SpeakerNameSession {
  speakerId: string;
  name: string | null;
  hadOverride: boolean;
  value: string;
}

export function nameBaseline(payload: DraftPayload, speaker: Speaker): SpeakerNameSession {
  const entry = payload.speaker_names.find(item => item.speaker_id === speaker.id);
  return { speakerId: speaker.id, name: speaker.displayName, hadOverride: !!entry, value: speaker.displayName ?? "" };
}

// Typing never enters the publication queue. Confirm one name, or explicitly
// drain the navigation/close barrier; keep all other names and text independent.
export function stageName(payload: DraftPayload, session: SpeakerNameSession): DraftPayload {
  if (Array.from(session.value).length > 200) throw Error("話者名は200文字までです。入力は保持しています。");
  const entry = payload.speaker_names.find(item => item.speaker_id === session.speakerId);
  const value = session.value || null;
  if (value === (entry ? entry.display_name : session.name)) return payload;
  return { ...payload, speaker_names: [...payload.speaker_names.filter(item => item.speaker_id !== session.speakerId),
    { speaker_id: session.speakerId, display_name: value }] };
}

export function cancelName(payload: DraftPayload, session: SpeakerNameSession): DraftPayload {
  const entry = payload.speaker_names.find(item => item.speaker_id === session.speakerId);
  if (!entry) return payload;
  return { ...payload, speaker_names: payload.speaker_names.flatMap(item => item !== entry ? [item]
    : session.hadOverride ? [{ ...entry, display_name: session.name }] : []) };
}

export function namesDirty(sessions: Record<string, SpeakerNameSession>, speakers: Speaker[]): boolean {
  const current = new Map(speakers.map(speaker => [speaker.id, speaker.displayName ?? ""]));
  return Object.values(sessions).some(session => current.has(session.speakerId) && current.get(session.speakerId) !== session.value);
}
