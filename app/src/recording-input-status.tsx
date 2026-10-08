import {invoke,isTauri} from "@tauri-apps/api/core";
import {createContext,useContext,useEffect,useState,type ReactNode} from "react";
import {useRecordingSession} from "./session";
import {InputLevelClock} from "./input-level-state";
import {RecordingInputSession,inputLabel,type InputKind,type VisibleInput,type InputNotice} from "./recording-input-state";
import {RecordingInputReceipts,type NoticeAttempt,type NoticeReception} from "./recording-input-receipts";
import {ToastNotice} from "./toast";
import "./recording-input-status.css";
type View={owner:string|null;microphone:VisibleInput;system:VisibleInput;notices:InputNotice[];reception:Record<InputKind,NoticeReception>;dismiss:(input:InputKind)=>void};
const unknown=()=>({microphone:{state:"unknown"} as VisibleInput,system:{state:"unknown"} as VisibleInput,notices:[] as InputNotice[]});
const empty=()=>({owner:null,...unknown(),reception:{microphone:"none",system:"none"} as Record<InputKind,NoticeReception>,dismiss:(_input:InputKind)=>{}});
const Context=createContext<View>(empty());
export function RecordingInputProvider({children,windowLabel}:{children:ReactNode;windowLabel:"main"|"recorder"}){
  const recording=useRecordingSession(),owner=recording.kind==="ready"&&["preparing","recording"].includes(recording.snapshot.phase)?recording.snapshot.sessionId:null;
  const [view,setView]=useState<View>(empty());
  useEffect(()=>{
    let disposed=false,inFlight=false,failed=true;const session=new RecordingInputSession(),clock=new InputLevelClock(performance.now(),Date.now());
    setView({...empty(),owner});try{session.reset(owner);}catch{return;}if(!owner||!isTauri())return;
    // A failed session bridge disposes this controller even if its WebView
    // survives. Reopening it must not impersonate a previously accepted receiver.
    const receipts=new RecordingInputReceipts(owner,crypto.randomUUID(),windowLabel);
    const now=()=>clock.read(performance.now(),Date.now());
    const paint=()=>{
      if(disposed)return;const at=now(),raw=failed?unknown():session.read(at),attempts=receipts.observe(raw.notices,at);
      setView({owner,...raw,...receipts.read(at),dismiss:input=>{if(!disposed){receipts.dismiss(input);paint();}}});
      for(const attempt of attempts)void receive(attempt);
    };
    const receive=async(attempt:NoticeAttempt)=>{
      try{
        const value=await invoke("receive_recording_input_notice",{sessionId:attempt.sessionId,noticeId:attempt.noticeId,recipientId:attempt.recipientId});
        if(disposed)return;paint();receipts.complete(attempt,value,now());paint();
      }catch{if(!disposed){receipts.fail(attempt);paint();}}
    };
    const poll=async()=>{if(disposed||inFlight)return;inFlight=true;const requestedAt=now();
      try{const report=await invoke("get_recording_input_status",{sessionId:owner});if(!disposed){session.accept(report,requestedAt);failed=false;}}
      catch{if(!disposed)failed=true;}finally{inFlight=false;paint();}
    };
    void poll();const painting=window.setInterval(paint,100),polling=window.setInterval(()=>void poll(),500);
    return()=>{disposed=true;window.clearInterval(painting);window.clearInterval(polling);};
  },[owner,windowLabel]);
  return <Context.Provider value={view.owner===owner?view:{...empty(),owner}}>{children}</Context.Provider>;
}
export function RecordingInputStatus({owner,input,compact=false}:{owner:string|null;input:InputKind;compact?:boolean}){
  const view=useContext(Context),value=view.owner===owner&&owner?view[input]:unknown()[input];
  const reception=view.owner===owner&&owner?view.reception[input]:"none",waiting=reception==="unconfirmed";
  const short={unknown:"確認不可",stale:"更新なし",connected:"接続済み",disconnected:"中断",reconnecting:"再取得中",unavailable:"取得不可",stopped:"停止"}[value.state];
  return <span className={compact?"mini-input-status":"recording-input-status"} data-input-kind={input} data-input-state={value.state} data-notice-reception={reception} aria-label={input==="microphone"?"マイクの取得状態":"PC音声の取得状態"} title={inputLabel(value,input)+(waiting?"。通知の受付結果を確認中です。取得や保存の成功を示す状態ではありません。":"")}>{compact?(input==="microphone"?"マイク: ":"PC: "):"接続状態: "}{compact?short:inputLabel(value,input)}{waiting&&!compact?" / 通知確認待ち":""}</span>;
}
export function RecordingMicrophoneStatus({owner,compact=false}:{owner:string|null;compact?:boolean}){return <RecordingInputStatus owner={owner} input="microphone" compact={compact}/>;}
export function RecordingCompactInputs({owner}:{owner:string|null}){return <span className="mini-inputs"><RecordingInputStatus owner={owner} input="microphone" compact/><RecordingInputStatus owner={owner} input="system" compact/></span>;}
const message=(notice:InputNotice)=>notice.input==="microphone"?(notice.kind==="interrupted"?"マイクの取得が中断しました。選択した機器と権限を確認してください。":"同じマイクの取得が復帰しました。欠損範囲は保存後に確認します。"):
  (notice.kind==="interrupted"?"PC音声の取得が中断しました。取得対象と権限を確認してください。":"同じ対象のPC音声取得が復帰しました。欠損範囲は保存後に確認します。");
export function RecordingInputNotice({windowLabel}:{windowLabel:"main"|"recorder"}){
  const view=useContext(Context),notice=(input:InputKind)=>view.notices.find(n=>n.input===input&&n.target===windowLabel)??null;
  const microphone=notice("microphone"),system=notice("system"),closeMicrophone=()=>view.dismiss("microphone"),closeSystem=()=>view.dismiss("system");
  // A read failure or stale interval can resolve a toast. Its later readback
  // must not deliver the same event again, even if the toast was not dismissed.
  if(windowLabel==="main"){
    const entries:[InputKind,InputNotice|null,()=>void][]=[["microphone",microphone,closeMicrophone],["system",system,closeSystem]];
    return <>{entries.map(([input,n,close])=><ToastNotice key={input} scope={view.owner??"recording-input"} type={`${input}-status`} message={n?message(n):""} level={n?.kind==="interrupted"?"warning":"info"} important={n?.kind==="interrupted"} eventId={n?.id} onClose={close}/>)}</>;
  }
  const current=[microphone,system].filter((n):n is InputNotice=>!!n),waiting=(["microphone","system"] as InputKind[]).filter(input=>view.reception[input]==="unconfirmed");
  if(!current.length&&!waiting.length)return null;
  const pending=waiting.map(input=>`${input==="microphone"?"マイク":"PC"}通知待ち`).join(" / ");
  if(!current.length)return <div className="mini-input-receipt-pending" role="status" title="通知の受付結果を確認中です。取得や保存の成功を示す状態ではありません。">{pending}</div>;
  const short=[pending,...current.map(n=>`${n.input==="microphone"?"マイク":"PC音声"}${n.kind==="interrupted"?"中断":"復帰"}`)].filter(Boolean).join(" / ");
  const warning=current.some(n=>n.kind==="interrupted"),aria=current.length===2?"入力通知を閉じる":microphone?"マイク通知を閉じる":"PC音声通知を閉じる";
  return <div className="mini-input-notice"><span role={warning?"alert":"status"} title={current.map(message).join(" ")+(waiting.length?" 通知の受付結果を確認中です。":"")}>{short}{waiting.length?"。":warning?"。詳細はメイン画面へ。":"。欠損範囲は保存後に確認。"}</span><button type="button" className="text-button" aria-label={aria} onClick={()=>{closeMicrophone();closeSystem();}}>×</button></div>;
}
