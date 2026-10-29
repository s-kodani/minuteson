import {invoke,isTauri} from "./ipc.ts";
import {useEffect,useState} from "react";
import {checkedMicrophoneSelectionStatus,microphoneFallbackMessage,type MicrophoneSelectionStatus} from "./microphone-selection-state";
import {toasts} from "./toast";
import {toastKey} from "./toast-state";

export function useRecordingMicrophoneSelection(owner:string|null):MicrophoneSelectionStatus|null{
  const [status,setStatus]=useState<MicrophoneSelectionStatus|null>(null);
  useEffect(()=>{
    setStatus(null);
    if(!owner||!isTauri())return;
    let disposed=false,inFlight=false,lastSuccess=0,lastRevision=0,lastRequestRevision=0,lastFallback:string|null=null;
    const key=toastKey(owner,"microphone-auto-fallback");
    const poll=async()=>{
      if(disposed||inFlight)return;
      inFlight=true;
      try{
        const next=checkedMicrophoneSelectionStatus(await invoke("get_recording_microphone_selection",{sessionId:owner}),owner);
        if(disposed||next.revision<lastRevision||next.requestRevision<lastRequestRevision)return;
        lastSuccess=performance.now();lastRevision=next.revision;lastRequestRevision=next.requestRevision;
        setStatus(current=>current?.revision===next.revision?current:next);
        const fallback=microphoneFallbackMessage(next),identity=fallback?`${next.source}:${next.fallbackReason}:${next.deviceUID}`:null;
        if(identity!==lastFallback){
          lastFallback=identity;
          if(fallback)toasts.show({key,level:"info",message:fallback,eventId:`${owner}:${next.revision}`});
          else toasts.resolve(key);
        }
      }catch{
        if(!disposed&&performance.now()-lastSuccess>1000)setStatus(null);
      }finally{inFlight=false;}
    };
    void poll();const timer=window.setInterval(()=>void poll(),500);
    return()=>{disposed=true;window.clearInterval(timer);toasts.resolve(key);};
  },[owner]);
  return status?.ownerId===owner?status:null;
}
