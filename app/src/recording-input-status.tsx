import {invoke,isTauri} from "@tauri-apps/api/core";
import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from "react";
import {useRecordingSession} from "./session";
import {InputLevelClock} from "./input-level-state";
import {RecordingInputSession,microphoneLabel,type VisibleInput,type InputNotice} from "./recording-input-state";
import {ToastNotice} from "./toast";
import "./recording-input-status.css";
type View={owner:string|null;microphone:VisibleInput;notice:InputNotice|null};
const unknown=()=>({microphone:{state:"unknown"} as VisibleInput,notice:null});
const Context=createContext<View>({owner:null,...unknown()});
export function RecordingInputProvider({children}:{children:ReactNode}){
  const recording=useRecordingSession(),owner=recording.kind==="ready"&&["preparing","recording"].includes(recording.snapshot.phase)?recording.snapshot.sessionId:null;
  const [view,setView]=useState<View>({owner:null,...unknown()});
  useEffect(()=>{
    let disposed=false,inFlight=false,failed=true;const session=new RecordingInputSession(),clock=new InputLevelClock(performance.now(),Date.now());
    setView({owner,...unknown()});try{session.reset(owner);}catch{return;}if(!owner||!isTauri())return;
    const now=()=>clock.read(performance.now(),Date.now());
    const paint=()=>{if(!disposed)setView({owner,...(failed?unknown():session.read(now()))});};
    const poll=async()=>{if(disposed||inFlight)return;inFlight=true;const requestedAt=now();
      try{const report=await invoke("get_recording_input_status",{sessionId:owner});if(!disposed){session.accept(report,requestedAt);failed=false;}}
      catch{if(!disposed)failed=true;}finally{inFlight=false;paint();}
    };
    void poll();const painting=window.setInterval(paint,100),polling=window.setInterval(()=>void poll(),500);
    return()=>{disposed=true;window.clearInterval(painting);window.clearInterval(polling);};
  },[owner]);
  return <Context.Provider value={view.owner===owner?view:{owner,...unknown()}}>{children}</Context.Provider>;
}
export function RecordingMicrophoneStatus({owner,compact=false}:{owner:string|null;compact?:boolean}){
  const view=useContext(Context),microphone=view.owner===owner&&owner?view.microphone:unknown().microphone;
  return <span className={compact?"mini-input-status":"recording-input-status"} data-input-state={microphone.state} aria-label="マイクの取得状態" title={microphoneLabel(microphone)}>{compact?"マイク: ":"接続状態: "}{microphoneLabel(microphone)}</span>;
}
const message=(notice:InputNotice)=>notice.kind==="interrupted"?"マイクの取得が中断しました。選択した機器と権限を確認してください。":"同じマイクの取得が復帰しました。欠損範囲は保存後に確認します。";
export function RecordingInputNotice({windowLabel}:{windowLabel:"main"|"recorder"}){
  const view=useContext(Context),notice=view.notice?.target===windowLabel?view.notice:null;
  const seen=useRef(new Set<string>()),[shown,setShown]=useState<InputNotice|null>(null);
  // A read failure or stale interval can resolve a toast. Its later readback
  // must not deliver the same event again, even if the toast was not dismissed.
  useEffect(()=>{if(!notice)setShown(null);else if(!seen.current.has(notice.id)){seen.current.add(notice.id);setShown(notice);}},[notice?.id]);
  const current=shown?.id===notice?.id?shown:null;
  if(windowLabel==="main")return <ToastNotice scope={view.owner??"recording-input"} type="microphone-status" message={current?message(current):""} level={current?.kind==="interrupted"?"warning":"info"} important={current?.kind==="interrupted"} eventId={current?.id} onClose={()=>setShown(null)}/>;
  if(!current)return null;
  return <div className="mini-input-notice"><span role={current.kind==="interrupted"?"alert":"status"}>{current.kind==="interrupted"?"マイク中断。詳細はメイン画面へ。":"同じマイクの取得が復帰しました。"}</span><button type="button" className="text-button" aria-label="マイク通知を閉じる" onClick={()=>setShown(null)}>×</button></div>;
}
