export interface PlaybackTimeline {
  owner: string; session: string; meetingId: string; analysisId: string | null;
  utteranceId: string | null; scope: "meeting" | "utterance"; timeUs: number; playing: boolean; trackKind: string;
}
let current: PlaybackTimeline | null = null;
const listeners = new Set<()=>void>();
export const playbackTimeline = {
  snapshot:()=>current,
  subscribe:(listener:()=>void)=>{listeners.add(listener);return ()=>{listeners.delete(listener);};},
  publish:(value:PlaybackTimeline)=>{
    if(!Number.isSafeInteger(value.timeUs)||value.timeUs<0)return;
    if(current&&Object.entries(value).every(([key,v])=>current?.[key as keyof PlaybackTimeline]===v))return;
    current={...value};for(const listener of listeners)listener();
  },
  release:(owner:string)=>{if(current?.owner!==owner)return;current=null;for(const listener of listeners)listener();},
};

export interface FollowRow { id: string; sourceUtteranceId?: string; trackId?: string; startUs: number; endUs: number }
export function playingRows(rows: readonly FollowRow[], timeline: PlaybackTimeline | null, meetingId: string, analysisId: string | null, trackKinds: ReadonlyMap<string,string> | null = null): number[] {
  if(!timeline?.playing||timeline.meetingId!==meetingId||!Number.isSafeInteger(timeline.timeUs)||timeline.timeUs<0)return [];
  if(timeline.scope==="utterance"&&timeline.analysisId!==analysisId)return [];
  const selected = timeline.trackKind === "both" ? ["mic","system"] : [timeline.trackKind];
  if(timeline.scope==="meeting"&&selected.some(kind=>!["mic","system","imported","processed"].includes(kind)))return [];
  const matches:number[]=[];
  rows.forEach((row,index)=>{
    if(!Number.isSafeInteger(row.startUs)||!Number.isSafeInteger(row.endUs)||row.startUs<0||row.endUs<=row.startUs
      ||row.startUs>timeline.timeUs||timeline.timeUs>=row.endUs)return;
    if(timeline.scope==="utterance" ? row.id===timeline.utteranceId||row.sourceUtteranceId===timeline.utteranceId
      : !!row.trackId&&!!trackKinds&&(trackKinds.get(row.trackId)==="processed" || selected.includes(trackKinds.get(row.trackId)??"")))matches.push(index);
  });
  return matches;
}
