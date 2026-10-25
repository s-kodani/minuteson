import { InfoPopover } from "./info-popover";
import {invoke,isTauri} from "@tauri-apps/api/core";
import {useEffect,useState} from "react";
import {InputLevelClock} from "./input-level-state";
import {WriteSession,type VisibleWrite} from "./recording-write-state";
const unknown=()=>({microphone:{state:"unavailable"} as VisibleWrite,system:{state:"unavailable"} as VisibleWrite});
export function useRecordingWrites(owner:string|null){
  const [view,setView]=useState(()=>({owner:null as string|null,...unknown()}));
  useEffect(()=>{
    let disposed=false,inFlight=false,failed=true;const session=new WriteSession(),clock=new InputLevelClock(performance.now(),Date.now());
    setView({owner,...unknown()});try{session.reset(owner);}catch{return;}if(!owner||!isTauri())return;
    const now=()=>clock.read(performance.now(),Date.now());
    const paint=()=>{if(!disposed)setView({owner,...(failed?unknown():session.read(now()))});};
    const poll=async()=>{if(disposed||inFlight)return;inFlight=true;const requestedAt=now();
      try{const value=await invoke("get_recording_writes",{sessionId:owner});if(!disposed){session.accept(value,requestedAt);failed=false;}}
      catch{if(!disposed)failed=true;}finally{inFlight=false;paint();}
    };
    void poll();const painting=window.setInterval(paint,100),polling=window.setInterval(()=>void poll(),500);
    return()=>{disposed=true;window.clearInterval(painting);window.clearInterval(polling);};
  },[owner]);
  return view.owner===owner?view:unknown();
}
function label(value:VisibleWrite){
  if(value.state==="written")return `書込み確認: ${(value.ageMs/1000).toFixed(1)}秒前`;
  return {waiting:"書込み待ち",unavailable:"書込みを確認できません",stale:"書込みの更新を確認できません"}[value.state];
}
export function RecordingWriteValue({value}:{value:VisibleWrite}){
  return <span data-write-state={value.state}>{label(value)}</span>;
}
export function RecordingWriteStatus({view}:{view:ReturnType<typeof useRecordingWrites>}){
  const confirmed=view.microphone.state==="written"&&view.system.state==="written";
  return <section aria-label="録音の書込み" className="recording-write-status" data-confirmed={confirmed}>
    <div className="heading-with-info"><p role="status">{confirmed?"この端末に保存中":"音声の書込みを確認しています"}</p><InfoPopover label="録音の書込み">録音ファイルにある音声と更新時刻を確認しています。入力レベルや接続状態だけでは保存中と表示しません。録音の保存完了は停止後に確認します。電源断による末尾の損失を保証する表示ではありません。</InfoPopover></div>
  </section>;
}
