import {invoke,isTauri} from "@tauri-apps/api/core";
import {useEffect,useState} from "react";
import {InputLevelClock,InputLevelSession,type VisibleLevel} from "./input-level-state";

const unavailable=()=>({microphone:{state:"unavailable"} as VisibleLevel,system:{state:"unavailable"} as VisibleLevel});
export function useRecordingLevels(owner:string|null){
  const [display,setDisplay]=useState<{owner:string|null;microphone:VisibleLevel;system:VisibleLevel}>({owner:null,...unavailable()});
  useEffect(()=>{
    let disposed=false,inFlight=false,failed=true;
    const session=new InputLevelSession();
    try{session.reset(owner);}catch{setDisplay({owner,...unavailable()});return;}
    const clock=new InputLevelClock(performance.now(),Date.now());
    const now=()=>clock.read(performance.now(),Date.now());
    setDisplay({owner,...unavailable()});
    if(!owner||!isTauri())return;
    const paint=()=>{if(!disposed)setDisplay({owner,...(failed?unavailable():session.read(now()))});};
    const poll=async()=>{
      if(disposed||inFlight)return;
      inFlight=true;
      // Include RPC latency in freshness, including the very first read.
      const requestedAt=now();
      try{
        const result=await invoke<unknown>("get_recording_levels",{sessionId:owner});
        if(!disposed){session.accept(result,requestedAt);failed=false;}
      }catch{if(!disposed)failed=true;}
      finally{inFlight=false;paint();}
    };
    void poll();
    const timer=window.setInterval(()=>{paint();void poll();},100);
    return()=>{disposed=true;window.clearInterval(timer);};
  },[owner]);
  return display.owner===owner?display:unavailable();
}

export function InputLevelMeter({label,level}:{label:string;level:VisibleLevel}){
  if(level.state!=="measured")return <span className="input-level" data-level-state={level.state}>{
    {waiting:"入力: 音声未受信",unavailable:"入力: 確認できません",stale:"入力: 測定値の更新が止まっています"}[level.state]
  }</span>;
  const db=(value:number|null)=>value===null?"無音":`${value.toFixed(1)} dBFS`;
  const text=`平均 ${db(level.rmsDbFS)} ／ピーク ${db(level.peakDbFS)}`;
  return <span className="input-level" data-level-state="measured">
    <meter min={0} max={1} value={Math.min(1,level.peak)} aria-label={`${label}のピーク入力レベル`} aria-valuetext={text}/>
    <span>{text}</span>
    {level.clipping&&<span className="input-level-clipping">音が大きすぎる可能性があります</span>}
  </span>;
}
