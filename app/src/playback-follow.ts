export interface PlaybackTimeline {
  owner: string; session: string; meetingId: string; analysisId: string | null;
  utteranceId: string | null; scope: "meeting" | "utterance"; timeUs: number; playing: boolean;
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

export interface FollowRow { id: string; sourceUtteranceId?: string; startUs: number; endUs: number }
// Never clear a filter or infer speech in a gap. Frozen minutes from another
// analysis must not move the current transcript at the same timestamp.
export function followingRow(rows: readonly FollowRow[], timeline: PlaybackTimeline | null, meetingId: string, analysisId: string | null): number | null {
  if(!timeline?.playing||timeline.meetingId!==meetingId)return null;
  if(timeline.scope==="utterance"&&timeline.analysisId!==analysisId)return null;
  const index=rows.findIndex(row=>row.startUs<=timeline.timeUs&&timeline.timeUs<row.endUs
    &&(timeline.scope==="meeting"||row.id===timeline.utteranceId||row.sourceUtteranceId===timeline.utteranceId));
  return index<0?null:index;
}

// Coordinates are relative to the one transcript scrollport, not the page.
// Tall rows stay put when their beginning is visible instead of oscillating.
export function followingScrollTop(view: {top:number;bottom:number;scrollTop:number;maxScroll:number}, row:{top:number;bottom:number}): number | null {
  if(view.bottom-view.top<=16||![view.top,view.bottom,view.scrollTop,view.maxScroll,row.top,row.bottom].every(Number.isFinite))return null;
  if(row.top>=view.top+8&&(row.bottom<=view.bottom-8||row.bottom-row.top>view.bottom-view.top-16&&row.top<view.bottom-8))return null;
  const next=Math.max(0,Math.min(view.maxScroll,view.scrollTop+row.top-view.top-8));
  return Math.abs(next-view.scrollTop)<1?null:next;
}
