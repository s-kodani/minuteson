import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";
import { ThemeReadNotice } from "./theme";
import { acceptRecordingDecision, recordingDecisionVisible, type RecordingDecision } from "./recording-confirmation-state";

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
    try{accept(await invoke<RecordingDecision>("request_recording_confirmation",{quit}));if(mounted.current)setRequestResolved(true);}
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
  return {snapshot,connected,error:error||readError,request,decide,refresh,requestBusy,requestResolved,decisionBusy,visible:recordingDecisionVisible(snapshot),stopSent:sent.current===snapshot?.id};
}

export function RecordingConfirmationWindow() {
  const decision=useRecordingConfirmation(null), value=decision.snapshot;
  const stopping=value?.phase==="stopping", failed=value?.phase==="failed";
  const quit=value?.quitAfterSave;
  const cancel=()=>decision.decide(false);
  return <main className="recording-confirmation-shell" aria-label="録音終了の確認ウィンドウ">
    {!decision.visible&&<p role="status">録音確認を読み込んでいます。</p>}
    {!decision.visible&&decision.error&&<><p role="alert">{decision.error}</p><button type="button" onClick={()=>void decision.refresh().catch(()=>{})}>状態を再確認</button></>}
    <CommonModal open={decision.visible} blocked={stopping||decision.decisionBusy||!decision.connected} dismissLabel={failed?"閉じる":"キャンセル"} dismissText={failed?undefined:"録音を続ける"} onDismiss={cancel} aria-labelledby="recording-decision-title">
      <h2 id="recording-decision-title">{failed?"録音の保存状態を確認":quit?"録音を終了してアプリを閉じますか？":value?.preparing?"録音準備を中止しますか？":"録音を終了しますか？"}</h2>
      <p>対象: {value?.meetingTitle}</p>
      <p>{value?.preparing?"録音準備を中止します。音声は保存されない場合があります。元の会議は保持します。":quit?"ここまでの音声を保存します。終了後は同じ録音を継続できません。保存済みの会議・音声・文書は保持します。":"ここまでの音声を保存し、準備済みモデルで解析を開始します。終了後は同じ録音を継続できません。モデル未準備でも音声は保持し、解析結果は確認待ちの候補として保存します。"}</p>
      {quit&&<p>録音と未保存編集の保存成功後に終了します。解析・モデル準備は安全な区切りで停止し、再起動後に解析を再実行できます。</p>}
      {stopping&&<p role="status">録音を停止・保存しています。保存完了を確認するまでお待ちください。</p>}
      {(value?.errorMessage||decision.error)&&<p role="alert" className="action-error">{value?.errorMessage||decision.error}</p>}
      <ThemeReadNotice />
      <div className="analysis-actions"><button type="button" className="stop-button" disabled={!decision.connected||decision.decisionBusy||decision.stopSent||value?.phase!=="confirming"} onClick={()=>void decision.decide(true)}>{quit?value?.preparing?"録音準備を中止して閉じる":"録音を終了して閉じる":value?.preparing?"録音準備を中止する":"録音を終了する"}</button><button type="button" className="small-secondary" disabled={decision.decisionBusy} onClick={()=>void decision.refresh().catch(()=>{})}>状態を再確認</button></div>
    </CommonModal>
  </main>;
}
