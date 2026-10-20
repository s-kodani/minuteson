import { invoke,isTauri } from "@tauri-apps/api/core";
import { useEffect,useState } from "react";
import { transcriptTrackKinds } from "./transcript-track-kinds";

export function useTranscriptTrackKinds(meetingId: string, analysisId: string | null) {
  const [read,setRead]=useState(0);
  const [state,setState]=useState<{meetingId:string;analysisId:string|null;kinds:ReadonlyMap<string,string>|null;error:string}>({meetingId,analysisId,kinds:null,error:""});
  useEffect(()=>{
    let disposed=false;
    setState({meetingId,analysisId,kinds:null,error:""});
    if(analysisId&&isTauri())void invoke("get_meeting_analysis_audio",{meetingId})
      .then(value=>{const kinds=transcriptTrackKinds(value);if(!disposed)setState({meetingId,analysisId,kinds,error:""});})
      .catch(cause=>{if(!disposed)setState({meetingId,analysisId,kinds:null,error:String(cause)});});
    return()=>{disposed=true;};
  },[meetingId,analysisId,read]);
  const matches=state.meetingId===meetingId&&state.analysisId===analysisId;
  return {kinds:matches?state.kinds:null,error:matches?state.error:"",retry:()=>setRead(value=>value+1)};
}
