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
  if(timeline.scope==="meeting"&&selected.some(kind=>!["mic","system","imported"].includes(kind)))return [];
  const matches:number[]=[];
  rows.forEach((row,index)=>{
    if(!Number.isSafeInteger(row.startUs)||!Number.isSafeInteger(row.endUs)||row.startUs<0||row.endUs<=row.startUs
      ||row.startUs>timeline.timeUs||timeline.timeUs>=row.endUs)return;
    if(timeline.scope==="utterance" ? row.id===timeline.utteranceId||row.sourceUtteranceId===timeline.utteranceId
      : !!row.trackId&&!!trackKinds&&selected.includes(trackKinds.get(row.trackId)??""))matches.push(index);
  });
  return matches;
}
// Never clear a filter or infer speech in a gap. Frozen minutes from another
// analysis must not move the current transcript at the same timestamp.
export function followingRow(rows: readonly FollowRow[], timeline: PlaybackTimeline | null, meetingId: string, analysisId: string | null, trackKinds: ReadonlyMap<string,string> | null = null): number | null {
  return playingRows(rows,timeline,meetingId,analysisId,trackKinds)[0]??null;
}

// Coordinates are relative to the one transcript scrollport, not the page.
// Tall rows stay put when their beginning is visible instead of oscillating.
export function followingScrollTop(view: {top:number;bottom:number;scrollTop:number;maxScroll:number}, row:{top:number;bottom:number}): number | null {
  if(view.bottom-view.top<=16||![view.top,view.bottom,view.scrollTop,view.maxScroll,row.top,row.bottom].every(Number.isFinite))return null;
  if(row.top>=view.top+8&&(row.bottom<=view.bottom-8||row.bottom-row.top>view.bottom-view.top-16&&row.top<view.bottom-8))return null;
  const next=Math.max(0,Math.min(view.maxScroll,view.scrollTop+row.top-view.top-8));
  return Math.abs(next-view.scrollTop)<1?null:next;
}
