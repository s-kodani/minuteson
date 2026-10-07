import { useLayoutEffect, useRef, useState, useSyncExternalStore, type RefObject, type PointerEvent, type KeyboardEvent } from "react";
import { nativeTextTarget, overlayOpen } from "./keyboard";
import { followingRow, followingScrollTop, playingRows, playbackTimeline, type FollowRow } from "./playback-follow";
import { readingPreferences } from "./theme-state";
import { useTheme } from "./theme";

export function usePlaybackFollowing({meetingId,analysisId,rows,page,pageSize,setPage,editing,active,port,trackKinds,highlightRows=rows}: {
  meetingId:string;analysisId:string|null;rows:readonly FollowRow[];page:number;pageSize:number;
  setPage:(value:number)=>void;editing:boolean;active:boolean;port:RefObject<HTMLDivElement|null>;
  trackKinds:ReadonlyMap<string,string>|null;
  highlightRows?:readonly FollowRow[];
}) {
  const timeline=useSyncExternalStore(playbackTimeline.subscribe,playbackTimeline.snapshot);
  const {state}=useTheme(),enabled=readingPreferences(state.snapshot?.payload).followPlayback;
  const [suspended,setSuspended]=useState(false),[resumeVersion,setResumeVersion]=useState(0);
  const session=useRef<{meetingId:string;key:string}|null>(null),previousEnabled=useRef(enabled);
  useLayoutEffect(()=>{
    if(enabled&&!previousEnabled.current)setSuspended(false);
    previousEnabled.current=enabled;
    if(timeline?.meetingId===meetingId&&timeline.session!==session.current?.key){session.current={meetingId,key:timeline.session};setSuspended(false);}
  },[enabled,timeline?.session,timeline?.meetingId,meetingId]);
  useLayoutEffect(()=>{
    if(!enabled||state.busy||state.uncertain||suspended||editing||!active||overlayOpen()||!port.current||document.getSelection()?.isCollapsed===false)return;
    if(port.current.closest(".transcript-card")?.contains(document.activeElement)&&nativeTextTarget(document.activeElement))return;
    const index=followingRow(rows,timeline,meetingId,analysisId,trackKinds);
    if(index===null)return;
    const targetPage=Math.floor(index/pageSize);
    if(page!==targetPage){setPage(targetPage);return;}
    const element=Array.from(port.current.querySelectorAll<HTMLElement>("[data-utterance-id]")).find(e=>e.dataset.utteranceId===rows[index].id);
    if(!element)return;
    const box=port.current.getBoundingClientRect(),row=element.getBoundingClientRect();
    const top=followingScrollTop({top:box.top,bottom:box.bottom,scrollTop:port.current.scrollTop,maxScroll:port.current.scrollHeight-port.current.clientHeight},row);
    if(top!==null)port.current.scrollTop=top;
  },[timeline,rows,page,pageSize,enabled,state.busy,state.uncertain,suspended,editing,active,meetingId,analysisId,resumeVersion,trackKinds]);
  // A track switch temporarily releases the timeline during preparation. A
  // manual scroll in that gap must still suspend this same playback session.
  const suspend=()=>{if(enabled&&(timeline?.meetingId===meetingId||session.current?.meetingId===meetingId))setSuspended(true);};
  const resume=()=>{setSuspended(false);setResumeVersion(v=>v+1);};
  const playingIds=new Set((active?playingRows(highlightRows,timeline,meetingId,analysisId,trackKinds):[]).map(index=>highlightRows[index].id));
  return {enabled,suspended,resume,suspend,playingIds,meetingPlaying:timeline?.meetingId===meetingId&&timeline.scope==="meeting"&&timeline.playing,handlers:{
    onWheelCapture:suspend,onTouchMoveCapture:suspend,
    onPointerDownCapture:(event:PointerEvent<HTMLDivElement>)=>{
      const box=event.currentTarget.getBoundingClientRect();
      if(event.clientX>=box.left+event.currentTarget.clientWidth)suspend();
    },
    onKeyDownCapture:(event:KeyboardEvent<HTMLDivElement>)=>{
      if(!nativeTextTarget(event.target)&&["ArrowUp","ArrowDown","PageUp","PageDown","Home","End"," "].includes(event.key))suspend();
    },
  }};
}
