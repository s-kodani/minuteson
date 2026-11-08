export interface Speaker { id: string; label: string; displayName: string | null }
export interface SpeakerIdentification {
  similarityMilli: number;
  runnerUpMilli: number | null;
  needsReview: boolean;
  reason: string;
}
export interface Utterance {
  speakerIdentification?: SpeakerIdentification | null;
  trackId?: string;
  id: string; startUs: number; endUs: number; recognizedText: string;
  effectiveText: string; attributionState: string; speakerIds: string[];
  recognitionIsSource?: boolean;
  sourceUtteranceId?: string;
}
export interface RecognitionReviewRange { trackId: string; startUs: number; endUs: number; reason: "repetition_after_recovery" }
export interface Analysis {
  speakerColorKeys?: Record<string, string>;
  reviewRanges?: RecognitionReviewRange[];
  id: string; meetingId: string; contentHash: string;
  speakers: Speaker[]; utterances: Utterance[];
}
export interface DraftPayload {
  schema_version: number;
  speaker_names: { speaker_id: string; display_name: string | null }[];
  utterances: { utterance_id: string; effective_text: string; speaker_ids: string[] }[];
  manual_speakers?: { id: string; label: string }[];
  speaker_merges?: { source_id: string; target_id: string }[];
  utterance_splits?: UtteranceSplit[];
}
export interface SplitPart {id:string;start_us:number;end_us:number;effective_text:string;speaker_ids:string[]}
export interface UtteranceSplit {source_utterance_id:string;source_effective_text:string;parts:SplitPart[]}
export interface DraftUtteranceAudioRequest {
  meetingId:string;analysisId:string;expectedContentHash:string;expectedDraft:{id:string;lockVersion:number};
  utteranceId:string;expectedStartUs:number;expectedEndUs:number;
}
const compareId = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
export function canonicalPayload(payload: DraftPayload): DraftPayload {
  if (payload.schema_version !== 4) throw Error("非対応の下書き形式です");
  const result: DraftPayload = {
    schema_version: 4, manual_speakers: [], speaker_merges: [], utterance_splits: [],
    speaker_names: [...payload.speaker_names].sort((a, b) => compareId(a.speaker_id, b.speaker_id)),
    utterances: payload.utterances.map(item => ({ ...item, speaker_ids: [...item.speaker_ids].sort(compareId) })).sort((a, b) => compareId(a.utterance_id, b.utterance_id)),
  };
  if (payload.manual_speakers?.length) result.manual_speakers = [...payload.manual_speakers].sort((a,b) => compareId(a.id,b.id));
  if (payload.speaker_merges?.length) result.speaker_merges = [...payload.speaker_merges].sort((a,b) => compareId(a.source_id,b.source_id));
  if (payload.utterance_splits?.length) result.utterance_splits=payload.utterance_splits.map(s=>({...s,parts:s.parts.map(p=>({...p,speaker_ids:[...p.speaker_ids].sort(compareId)}))})).sort((a,b)=>compareId(a.source_utterance_id,b.source_utterance_id));
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
  const utterances = base.utterances.flatMap(u => {
      const split=payload.utterance_splits?.find(s=>s.source_utterance_id===u.id);
      if(split) return split.parts.map(p=>{
        const edit=edits.get(p.id),speakerIds=mergedIds(payload,edit?.speaker_ids ?? p.speaker_ids);
        return {...u,id:p.id,startUs:p.start_us,endUs:p.end_us,effectiveText:edit?.effective_text ?? p.effective_text,speakerIds,
          speakerIdentification:undefined,recognitionIsSource:true,sourceUtteranceId:u.id,attributionState:speakerIds.length===0?"unknown":speakerIds.length===1?"assigned":"overlap"};
      });
      const edit = edits.get(u.id);
      const speakerIds = mergedIds(payload,edit?.speaker_ids ?? u.speakerIds);
      const changed = speakerIds.length!==u.speakerIds.length || speakerIds.some(id=>!u.speakerIds.includes(id));
      return [{...u,effectiveText:edit?.effective_text ?? u.effectiveText,speakerIds,
        ...(changed?{speakerIdentification:undefined}:{}),
        attributionState:speakerIds.length===0?"unknown":speakerIds.length===1?"assigned":"overlap"}];
    });
  // Preserve the stored sequence unless a structural split needs a new one.
  // For splits, match the native projection's time/track/ID ordering.
  if (payload.utterance_splits?.length) utterances.sort((a,b)=>a.startUs-b.startUs||compareId(a.trackId??"",b.trackId??"")||compareId(a.id,b.id));
  return { ...base,
    speakers: [...base.speakers,...(payload.manual_speakers ?? []).map(s => ({...s,displayName:null}))].map(s => ({...s,displayName:names.has(s.id)?names.get(s.id)!:s.displayName})),
    utterances,
  };
}
export function activeSpeakers(view: Analysis, payload: DraftPayload | null): Speaker[] {
  const hidden = new Set(payload?.speaker_merges?.map(m=>m.source_id) ?? []);
  return view.speakers.filter(s=>!hidden.has(s.id));
}
export function mergeSpeaker(base: Analysis, payload: DraftPayload, sourceId: string, targetId: string): DraftPayload {
  const active = new Set(activeSpeakers(projectTranscript(base,payload),payload).map(s=>s.id));
  if (sourceId === targetId || !active.has(sourceId) || !active.has(targetId)) throw new Error("統合元と統合先には別の有効な話者を選んでください。");
  return canonicalPayload({...payload,schema_version:4,speaker_merges:[
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
  return canonicalPayload({...payload,schema_version:4,
    manual_speakers:[...(payload.manual_speakers ?? []),{id,label:`手動話者 ${(payload.manual_speakers?.length ?? 0)+1}`}],
    speaker_names:[...payload.speaker_names,{speaker_id:id,display_name:name || null}],
    utterances:utterance?[...payload.utterances.filter(e=>e.utterance_id!==utterance.id),{utterance_id:utterance.id,effective_text:utterance.effectiveText,speaker_ids:[id]}]:payload.utterances,
  });
}

export function textPieces(text:string):string[] {
  return Array.from(new Intl.Segmenter("ja",{granularity:"grapheme"}).segment(text),s=>s.segment);
}
export function splitPosition(startUs:number,endUs:number,seconds:string):number {
  if(!/^\d+(?:\.\d{1,6})?$/.test(seconds))throw new Error("発言先頭からの秒数を小数6桁以内で指定してください。");
  const [whole,fraction=""]=seconds.split(".");
  const offset=Number(whole)*1_000_000+Number(fraction.padEnd(6,"0"));
  const at=startUs+offset;
  if(!Number.isSafeInteger(at)||at<=startUs||at>=endUs)throw new Error("分割時刻は元の発言の内側に指定してください。");
  return at;
}
export function splitUtterance(base:Analysis,payload:DraftPayload,utteranceId:string,atUs:number,textOffset:number,ids:[string,string],assignments?:[string[],string[]]):DraftPayload {
  const view=projectTranscript(base,payload),utterance=view.utterances.find(u=>u.id===utteranceId);
  if(!utterance)throw new Error("分割対象の発言がありません。");
  if(!Number.isSafeInteger(atUs)||atUs<=utterance.startUs||atUs>=utterance.endUs)throw new Error("分割時刻は元の発言の内側に指定してください。");
  const pieces=textPieces(utterance.effectiveText);
  if(!Number.isInteger(textOffset)||textOffset<1||textOffset>=pieces.length)throw new Error("本文の切れ目を指定してください。空の部分は作れません。");
  if(ids[0]===ids[1]||ids.some(id=>!/^split:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id)||view.utterances.some(u=>u.id===id)))throw new Error("分割先のIDが不正です。");
  if(view.utterances.length>=100_000)throw new Error("分割後の発言は100000件までです。");
  const active=new Set(activeSpeakers(view,payload).map(s=>s.id));
  const assigned=assignments ?? [utterance.speakerIds,utterance.speakerIds];
  if(assigned.some(group=>group.some(id=>!active.has(id))))throw new Error("分割先の話者が不正です。");
  const originalId=utterance.sourceUtteranceId ?? utterance.id;
  const existing=payload.utterance_splits?.find(s=>s.source_utterance_id===originalId);
  const parent=base.utterances.find(u=>u.id===originalId)!;
  const currentParts=existing?view.utterances.filter(u=>u.sourceUtteranceId===originalId):[utterance];
  const parts=currentParts.flatMap(u=>u.id===utteranceId?[
    {id:ids[0],start_us:u.startUs,end_us:atUs,effective_text:pieces.slice(0,textOffset).join(""),speaker_ids:assigned[0]},
    {id:ids[1],start_us:atUs,end_us:u.endUs,effective_text:pieces.slice(textOffset).join(""),speaker_ids:assigned[1]},
  ]:[{id:u.id,start_us:u.startUs,end_us:u.endUs,effective_text:u.effectiveText,speaker_ids:u.speakerIds}]);
  if(parts.length>1000)throw new Error("1発言の分割は1000部分までです。");
  const sourceText=parts.map(p=>p.effective_text).join("");
  const removed=new Set([originalId,...currentParts.map(u=>u.id)]);
  const sourceEdit=payload.utterances.find(e=>e.utterance_id===originalId);
  return canonicalPayload({...payload,schema_version:4,
    utterance_splits:[...(payload.utterance_splits ?? []).filter(s=>s.source_utterance_id!==originalId),{source_utterance_id:originalId,source_effective_text:sourceText,parts}],
    utterances:[...payload.utterances.filter(e=>!removed.has(e.utterance_id)),{utterance_id:originalId,effective_text:sourceText,speaker_ids:sourceEdit?.speaker_ids ?? parent.speakerIds}],
  });
}

export function filterUtterances(view:Analysis,speakerId:string,query:string):Utterance[] {
  const text=query.trim().toLocaleLowerCase();
  return view.utterances.filter(u=>(speakerId===""||speakerId==="unknown"?speakerId===""||u.speakerIds.length===0:u.speakerIds.includes(speakerId))&&(!text||u.effectiveText.toLocaleLowerCase().includes(text)));
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
