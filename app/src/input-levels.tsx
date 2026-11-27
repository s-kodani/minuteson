import {isTauri} from "./ipc.ts";
import {useEffect,useState} from "react";
import type {VisibleLevel} from "./input-level-state";
import {startInputLevelPoller,type LevelDisplay} from "./input-level-poller.ts";

type Window="main"|"recorder";
const unavailable=()=>({microphone:{state:"unavailable"} as VisibleLevel,system:{state:"unavailable"} as VisibleLevel});
export function useRecordingLevels(owner:string|null,windowLabel:Window){
  return useOwnedLevels(owner,"recording",windowLabel);
}
// The 5-second capture test is shown only in the main window.
export function useCaptureTestLevels(owner:string|null){
  return useOwnedLevels(owner,"test","main");
}
function useOwnedLevels(owner:string|null,kind:"recording"|"test",windowLabel:Window){
  const [display,setDisplay]=useState<LevelDisplay>({owner:null,revision:null,...unavailable()});
  useEffect(()=>{
    setDisplay({owner,revision:null,...unavailable()});
    if(!owner||!isTauri())return;
    return startInputLevelPoller({owner,kind,windowLabel,onUpdate:setDisplay});
  },[owner,kind,windowLabel]);
  return display.owner===owner?display:{revision:null,...unavailable()};
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
