import {invoke,isTauri} from "@tauri-apps/api/core";
import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from "react";
import {useRecordingSession} from "./session";
import {InputLevelClock} from "./input-level-state";
import {RecordingInputSession,inputLabel,type InputKind,type VisibleInput,type InputNotice} from "./recording-input-state";
import {ToastNotice} from "./toast";
import "./recording-input-status.css";
type View={owner:string|null;microphone:VisibleInput;system:VisibleInput;notices:InputNotice[]};
const unknown=()=>({microphone:{state:"unknown"} as VisibleInput,system:{state:"unknown"} as VisibleInput,notices:[] as InputNotice[]});
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
export function RecordingInputStatus({owner,input,compact=false}:{owner:string|null;input:InputKind;compact?:boolean}){
  const view=useContext(Context),value=view.owner===owner&&owner?view[input]:unknown()[input];
  const short={unknown:"確認不可",stale:"更新なし",connected:"接続済み",disconnected:"中断",reconnecting:"再取得中",unavailable:"取得不可",stopped:"停止"}[value.state];
  return <span className={compact?"mini-input-status":"recording-input-status"} data-input-kind={input} data-input-state={value.state} aria-label={input==="microphone"?"マイクの取得状態":"PC音声の取得状態"} title={inputLabel(value,input)}>{compact?(input==="microphone"?"マイク: ":"PC: "):"接続状態: "}{compact?short:inputLabel(value,input)}</span>;
}
export function RecordingMicrophoneStatus({owner,compact=false}:{owner:string|null;compact?:boolean}){return <RecordingInputStatus owner={owner} input="microphone" compact={compact}/>;}
export function RecordingCompactInputs({owner}:{owner:string|null}){return <span className="mini-inputs"><RecordingInputStatus owner={owner} input="microphone" compact/><RecordingInputStatus owner={owner} input="system" compact/></span>;}
const message=(notice:InputNotice)=>notice.input==="microphone"?(notice.kind==="interrupted"?"マイクの取得が中断しました。選択した機器と権限を確認してください。":"同じマイクの取得が復帰しました。欠損範囲は保存後に確認します。"):
  (notice.kind==="interrupted"?"PC音声の取得が中断しました。取得対象と権限を確認してください。":"同じ対象のPC音声取得が復帰しました。欠損範囲は保存後に確認します。");
function useShownNotice(owner:string|null,notice:InputNotice|null){
  const seen=useRef(new Set<string>()),priorOwner=useRef(owner),[shown,setShown]=useState<InputNotice|null>(null);
  useEffect(()=>{
    if(priorOwner.current!==owner){priorOwner.current=owner;seen.current.clear();setShown(null);}
    if(!notice)setShown(null);else if(!seen.current.has(notice.id)){seen.current.add(notice.id);setShown(notice);}
  },[notice?.id,owner]);
  return [shown?.id===notice?.id?shown:null,()=>setShown(null)] as const;
}
export function RecordingInputNotice({windowLabel}:{windowLabel:"main"|"recorder"}){
  const view=useContext(Context),notice=(input:InputKind)=>view.notices.find(n=>n.input===input&&n.target===windowLabel)??null;
  const [microphone,closeMicrophone]=useShownNotice(view.owner,notice("microphone")),[system,closeSystem]=useShownNotice(view.owner,notice("system"));
  // A read failure or stale interval can resolve a toast. Its later readback
  // must not deliver the same event again, even if the toast was not dismissed.
  if(windowLabel==="main"){
    const entries:[InputKind,InputNotice|null,()=>void][]=[["microphone",microphone,closeMicrophone],["system",system,closeSystem]];
    return <>{entries.map(([input,n,close])=><ToastNotice key={input} scope={view.owner??"recording-input"} type={`${input}-status`} message={n?message(n):""} level={n?.kind==="interrupted"?"warning":"info"} important={n?.kind==="interrupted"} eventId={n?.id} onClose={close}/>)}</>;
  }
  const current=[microphone,system].filter((n):n is InputNotice=>!!n);if(!current.length)return null;
  const short=current.map(n=>`${n.input==="microphone"?"マイク":"PC音声"}${n.kind==="interrupted"?"中断":"復帰"}`).join(" / ");
  const warning=current.some(n=>n.kind==="interrupted"),aria=current.length===2?"入力通知を閉じる":microphone?"マイク通知を閉じる":"PC音声通知を閉じる";
  return <div className="mini-input-notice"><span role={warning?"alert":"status"} title={current.map(message).join(" ")}>{short}{warning?"。詳細はメイン画面へ。":"。欠損範囲は保存後に確認。"}</span><button type="button" className="text-button" aria-label={aria} onClick={()=>{closeMicrophone();closeSystem();}}>×</button></div>;
}
