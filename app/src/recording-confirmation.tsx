import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { acceptRecordingDecision, recordingDecisionVisible, type RecordingDecision } from "./recording-confirmation-state";
import { useRecorderOperationPublisher } from "./recorder-operation-notices";

export function useRecordingConfirmation(origin: "main" | "recorder" | null) {
  const [snapshot, setSnapshot] = useState<RecordingDecision | null>(null), [connected,setConnected]=useState(false), [error,setError]=useState("");
  const [readError,setReadError]=useState("");
  const [requestResolved,setRequestResolved]=useState(false);
  const current=useRef<RecordingDecision|null>(null), mounted=useRef(true), requesting=useRef(false), deciding=useRef(false), sent=useRef<string|null>(null), opener=useRef<HTMLElement|null>(null);
  const [requestBusy,setRequestBusy]=useState(false), [decisionBusy,setDecisionBusy]=useState(false);
  const accept=(next:RecordingDecision)=>{
    if(!mounted.current)return;
    const value=acceptRecordingDecision(current.current,next);
    if(current.current?.id!==value.id)setError("");
    current.current=value;setSnapshot(value);setConnected(true);setReadError("");
  };
  const refresh=async()=>{
    try{accept(await invoke<RecordingDecision>("get_recording_confirmation"));}
    catch(cause){if(mounted.current){setConnected(false);setReadError(String(cause));}throw cause;}
  };
  const refreshRef=useRef(refresh);refreshRef.current=refresh;
  useEffect(()=>{
    mounted.current=true;if(!isTauri())return;
    let disposed=false, reading=false;const stops:(()=>void)[]=[];
    const read=async()=>{if(disposed||reading)return;reading=true;try{await refreshRef.current();}catch{}finally{reading=false;}};
    void listen<RecordingDecision>("recording-confirmation-changed",event=>{if(!disposed)try{accept(event.payload);}catch(cause){setConnected(false);setReadError(String(cause));}}).then(stop=>{if(disposed)stop();else{stops.push(stop);void read();}}).catch(()=>void read());
    if(origin)void listen<string>("recording-confirmation-request-error",event=>{if(!disposed&&typeof event.payload==="string")setError(event.payload);}).then(stop=>{if(disposed)stop();else stops.push(stop);}).catch(cause=>{if(!disposed)setReadError(String(cause));});
    const timer=window.setInterval(()=>void read(),500);window.addEventListener("focus",read);
    return()=>{disposed=true;mounted.current=false;stops.forEach(stop=>stop());window.clearInterval(timer);window.removeEventListener("focus",read);};
  },[]);
  useEffect(()=>{
    if(snapshot?.phase!=="cancelled"||snapshot.origin!==origin||!opener.current)return;
    const target=opener.current;
    if(!target.isConnected||target.matches(":disabled"))return;
    target.focus({preventScroll:true});opener.current=null;
  },[snapshot?.revision,requestBusy,origin]);
  const request=async(quit=false,source?:HTMLElement)=>{
    if(requesting.current)return;requesting.current=true;setRequestBusy(true);setError("");setRequestResolved(false);
    // macOS mouse activation does not necessarily focus its button. Keep
    // the actual initiating control, not whichever field happened to be active.
    opener.current=source??(document.activeElement instanceof HTMLElement?document.activeElement:null);
    const rect=source?.getBoundingClientRect();
    const anchor=rect?{x:Math.round(rect.x),y:Math.round(rect.y),width:Math.round(rect.width),height:Math.round(rect.height)}:null;
    try{accept(await invoke<RecordingDecision>("request_recording_confirmation",{quit,anchor}));if(mounted.current)setRequestResolved(true);}
    catch(cause){if(mounted.current)setError(String(cause));if(quit)throw cause;}
    finally{requesting.current=false;if(mounted.current)setRequestBusy(false);}
  };
  const decide=async(stop:boolean)=>{
    const value=current.current;if(deciding.current||!value?.id||!connected||value.phase==="stopping"||(stop&&(value.phase!=="confirming"||sent.current===value.id)))return;
    deciding.current=true;setDecisionBusy(true);setError("");if(stop)sent.current=value.id;
    try{accept(await invoke<RecordingDecision>("decide_recording_confirmation",{id:value.id,stop,expectedPreparing:value.preparing}));}
    catch(cause){if(mounted.current)setError(`受付状態を確認してください。停止は自動再送していません: ${String(cause)}`);try{await refreshRef.current();if(current.current?.id===value.id&&current.current.phase==="confirming"&&current.current.preparing!==value.preparing)sent.current=null;}catch{}}
    finally{deciding.current=false;if(mounted.current)setDecisionBusy(false);}
  };
  return {snapshot,connected,error,readError,request,decide,refresh,requestBusy,requestResolved,decisionBusy,visible:recordingDecisionVisible(snapshot),stopSent:sent.current===snapshot?.id};
}

export function RecordingConfirmationWindow() {
  const decision=useRecordingConfirmation(null), value=decision.snapshot;
  useRecorderOperationPublisher("confirmation", decision.error, !decision.requestBusy, decision.connected && !decision.error);
  const stopping=value?.phase==="stopping", failed=value?.phase==="failed";
  const quit=value?.quitAfterSave;
  const cancel=()=>decision.decide(false);
  const cancelButton=useRef<HTMLButtonElement>(null), focusedId=useRef<string|null>(null);
  useEffect(()=>{
    if(!decision.visible||!value?.id||focusedId.current===value.id)return;
    focusedId.current=value.id;
    cancelButton.current?.focus({preventScroll:true});
  },[decision.visible,value?.id]);
  useEffect(()=>{
    if(!decision.visible)return;
    const onKeyDown=(event:KeyboardEvent)=>{
      if(event.key!=="Escape"||event.isComposing)return;
      event.preventDefault();
      if(!stopping&&!decision.decisionBusy&&decision.connected)void cancel();
    };
    window.addEventListener("keydown",onKeyDown);
    return()=>window.removeEventListener("keydown",onKeyDown);
  },[decision.visible,stopping,decision.decisionBusy,decision.connected]);
  return <main className="recording-confirmation-shell" aria-label="録音終了の確認ウィンドウ">
    {!decision.visible&&<p role="status" className="recording-confirmation-loading">録音確認を読み込んでいます。</p>}

    {decision.visible&&<section role="dialog" aria-modal="false" aria-labelledby="recording-decision-title" className="recording-confirmation-popover">
      <h2 id="recording-decision-title">{failed?"録音の保存状態を確認":quit?"録音を終了してアプリを閉じますか？":value?.preparing?"録音準備を中止しますか？":"録音を終了しますか？"}</h2>
      <p className="recording-confirmation-meeting" title={value?.meetingTitle??undefined}>{value?.meetingTitle}</p>
      <p>{value?.preparing?"録音準備を中止します。音声が保存されない場合があります。":quit?"ここまでの音声と未保存の編集を保存してからアプリを閉じます。保存後、この録音は再開できません。":"ここまでの音声を保存して解析を開始します。保存後、この録音は再開できません。"}</p>
      {stopping&&<p role="status">録音を停止・保存しています。保存完了を確認するまでお待ちください。</p>}

      <div className="analysis-actions"><button ref={cancelButton} type="button" className="small-secondary" disabled={stopping||decision.decisionBusy||!decision.connected} onClick={cancel}>{failed?"確認を閉じる":"録音を続ける"}</button><button type="button" className="stop-button" disabled={!decision.connected||decision.decisionBusy||decision.stopSent||value?.phase!=="confirming"} onClick={()=>void decision.decide(true)}>{quit?value?.preparing?"録音準備を中止して閉じる":"録音を終了して閉じる":value?.preparing?"録音準備を中止する":"録音を終了する"}</button></div>
    </section>}
  </main>;
}
