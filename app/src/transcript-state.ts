export interface Speaker { id: string; label: string; displayName: string | null }
export interface Utterance {
  id: string; startUs: number; endUs: number; recognizedText: string;
  effectiveText: string; attributionState: string; speakerIds: string[];
}
export interface Analysis {
  id: string; meetingId: string; contentHash: string;
  speakers: Speaker[]; utterances: Utterance[];
}
export interface DraftPayload {
  schema_version: number;
  speaker_names: { speaker_id: string; display_name: string | null }[];
  utterances: { utterance_id: string; effective_text: string; speaker_ids: string[] }[];
  manual_speakers?: { id: string; label: string }[];
  speaker_merges?: { source_id: string; target_id: string }[];
}
const compareId = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
export function canonicalPayload(payload: DraftPayload): DraftPayload {
  const result: DraftPayload = {
    schema_version: payload.schema_version,
    speaker_names: [...payload.speaker_names].sort((a, b) => compareId(a.speaker_id, b.speaker_id)),
    utterances: payload.utterances.map(item => ({ ...item, speaker_ids: [...item.speaker_ids].sort(compareId) })).sort((a, b) => compareId(a.utterance_id, b.utterance_id)),
  };
  if (payload.manual_speakers?.length) result.manual_speakers = [...payload.manual_speakers].sort((a,b) => compareId(a.id,b.id));
  if (payload.speaker_merges?.length) result.speaker_merges = [...payload.speaker_merges].sort((a,b) => compareId(a.source_id,b.source_id));
  return result;
}
export function mergedIds(payload: DraftPayload | null, ids: string[]): string[] {
  const mapping = new Map(payload?.speaker_merges?.map(m => [m.source_id,m.target_id]) ?? []);
  return [...new Set(ids.map(id => mapping.get(id) ?? id))].sort(compareId);
}
// A projection for editing/display only. Never use it as an immutable analysis
// identity or send its changed contents with the base contentHash to a read RPC.
export function projectTranscript(base: Analysis, payload: DraftPayload | null): Analysis {
  if (!payload) return base;
  const names = new Map(payload.speaker_names.map(e => [e.speaker_id,e.display_name]));
  const edits = new Map(payload.utterances.map(e => [e.utterance_id,e]));
  return { ...base,
    speakers: [...base.speakers,...(payload.manual_speakers ?? []).map(s => ({...s,displayName:null}))].map(s => ({...s,displayName:names.has(s.id)?names.get(s.id)!:s.displayName})),
    utterances: base.utterances.map(u => {
      const edit = edits.get(u.id);
      const speakerIds = mergedIds(payload,edit?.speaker_ids ?? u.speakerIds);
      return {...u,effectiveText:edit?.effective_text ?? u.effectiveText,speakerIds,
        attributionState:speakerIds.length===0?"unknown":speakerIds.length===1?"assigned":"overlap"};
    }),
  };
}
export function activeSpeakers(view: Analysis, payload: DraftPayload | null): Speaker[] {
  const hidden = new Set(payload?.speaker_merges?.map(m=>m.source_id) ?? []);
  return view.speakers.filter(s=>!hidden.has(s.id));
}
export function mergeSpeaker(base: Analysis, payload: DraftPayload, sourceId: string, targetId: string): DraftPayload {
  const active = new Set(activeSpeakers(projectTranscript(base,payload),payload).map(s=>s.id));
  if (sourceId === targetId || !active.has(sourceId) || !active.has(targetId)) throw new Error("統合元と統合先には別の有効な話者を選んでください。");
  return canonicalPayload({...payload,schema_version:2,speaker_merges:[
    ...(payload.speaker_merges ?? []).map(m=>({...m,target_id:m.target_id===sourceId?targetId:m.target_id})),
    {source_id:sourceId,target_id:targetId},
  ]});
}
export function addSpeaker(base: Analysis, payload: DraftPayload, id: string, name: string, utteranceId?: string): DraftPayload {
  const view = projectTranscript(base,payload);
  if (view.speakers.length>=100) throw new Error("話者は推定元を含め100件までです。");
  if (!/^manual:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id) || view.speakers.some(s=>s.id===id)) throw new Error("追加話者のIDが不正です。");
  if ([...name].length>200) throw new Error("話者名は200文字までです。");
  const utterance = utteranceId ? view.utterances.find(u=>u.id===utteranceId) : null;
  if (utteranceId && !utterance) throw new Error("発言が見つかりません。");
  return canonicalPayload({...payload,schema_version:2,
    manual_speakers:[...(payload.manual_speakers ?? []),{id,label:`手動話者 ${(payload.manual_speakers?.length ?? 0)+1}`}],
    speaker_names:[...payload.speaker_names,{speaker_id:id,display_name:name || null}],
    utterances:utterance?[...payload.utterances.filter(e=>e.utterance_id!==utterance.id),{utterance_id:utterance.id,effective_text:utterance.effectiveText,speaker_ids:[id]}]:payload.utterances,
  });
}
export function assignSpeakers(base: Analysis, payload: DraftPayload, selected: string[], speakerIds: string[]): DraftPayload {
  const view = projectTranscript(base,payload);
  const valid = new Set(activeSpeakers(view,payload).map(s=>s.id));
  if (speakerIds.some(id=>!valid.has(id))) throw new Error("割り当て先の話者が不正です。");
  const ids = new Set(selected);
  if (!ids.size || [...ids].some(id=>!view.utterances.some(u=>u.id===id))) throw new Error("変更する発言を選んでください。");
  return canonicalPayload({...payload,utterances:[...payload.utterances.filter(e=>!ids.has(e.utterance_id)),
    ...view.utterances.filter(u=>ids.has(u.id)).map(u=>({utterance_id:u.id,effective_text:u.effectiveText,speaker_ids:[...new Set(speakerIds)]})),
  ]});
}
