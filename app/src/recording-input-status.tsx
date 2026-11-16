import {invoke,isTauri} from "./ipc.ts";
import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from "react";
import {useRecordingSession} from "./session";
import {InputLevelClock} from "./input-level-state";
import {RecordingInputSession,inputLabel,type InputKind,type VisibleInput} from "./recording-input-state";
import {InputJournalSession,InputJournalFailures} from "./input-journal-state";
import "./recording-input-status.css";
import {InputDiagnosticLog} from "./input-diagnostic-log";
type View={owner:string|null;microphone:VisibleInput;system:VisibleInput};
const unknown=()=>({microphone:{state:"unknown"} as VisibleInput,system:{state:"unknown"} as VisibleInput});
const empty=():View=>({owner:null,...unknown()});
const Context=createContext<View>(empty());
export function RecordingInputProvider({children,windowLabel}:{children:ReactNode;windowLabel:"main"|"recorder"}){
  const recording=useRecordingSession(),owner=recording.kind==="ready"&&["preparing","recording"].includes(recording.snapshot.phase)?recording.snapshot.sessionId:null;
  const phaseRef=useRef(recording.kind==="ready"?recording.snapshot.phase:"idle");phaseRef.current=recording.kind==="ready"?recording.snapshot.phase:"idle";
  const [view,setView]=useState<View>(empty());
  useEffect(()=>{
    let disposed=false,inFlight=false,failed=true;const session=new RecordingInputSession(),clock=new InputLevelClock(performance.now(),Date.now());
    setView({...empty(),owner});try{session.reset(owner);}catch{return;}if(!owner||!isTauri())return;
    const journal=new InputJournalSession(owner),journalFailures=new InputJournalFailures(),diagnosticLog=new InputDiagnosticLog();let journalInFlight=false;
    const now=()=>clock.read(performance.now(),Date.now());
    const logJournal=(at:number)=>{
      const view=journal.read(at);
      const failure=journalFailures.read(view,phaseRef.current,at);
      if(failure){
        const message=failure==="failed"
          ?"マイク・PC音声の中断・再開履歴を保存できませんでした。"
          :"マイク・PC音声の中断・再開履歴に不整合が見つかり、保存を停止しました。";
        diagnosticLog.report("live-journal-notice",owner,message,{state:failure});
      }else if(view.state==="confirmed")diagnosticLog.report("live-journal-notice",owner,null);
    };
    const paint=()=>{
      if(disposed)return;const at=now(),raw=failed?unknown():session.read(at);
      logJournal(at);
      setView({owner,microphone:raw.microphone,system:raw.system});
    };
    const poll=async()=>{if(disposed||inFlight)return;inFlight=true;const requestedAt=now();
      try{const report=await invoke("get_recording_input_status",{sessionId:owner});if(!disposed){session.accept(report,requestedAt);failed=false;diagnosticLog.report("live-status",owner,null);}}
      catch(error){if(!disposed){failed=true;diagnosticLog.report("live-status",owner,error);}}finally{inFlight=false;paint();}
    };
    const pollJournal=async()=>{if(disposed||journalInFlight)return;journalInFlight=true;const at=now();
      try{const value=await invoke("get_recording_input_history_status",{sessionId:owner});if(!disposed){journal.accept(value,at);diagnosticLog.report("live-journal",owner,null);}}
      catch(error){if(!disposed){journal.fail();diagnosticLog.report("live-journal",owner,error,{command:"get_recording_input_history_status"});}}finally{journalInFlight=false;if(!disposed)paint();}
    };
    void poll();void pollJournal();const painting=window.setInterval(paint,100),polling=window.setInterval(()=>{void poll();void pollJournal();},500);
    return()=>{disposed=true;window.clearInterval(painting);window.clearInterval(polling);};
  },[owner,windowLabel]);
  return <Context.Provider value={view.owner===owner?view:{...empty(),owner}}>{children}</Context.Provider>;
}
export function useRecordingInputValue(owner:string|null,input:InputKind):VisibleInput{
  const view=useContext(Context);
  return view.owner===owner&&owner?view[input]:unknown()[input];
}
export function RecordingInputStatus({owner,input,indicator=false,showPrefix=true}:{owner:string|null;input:InputKind;indicator?:boolean;showPrefix?:boolean}){
  const view=useContext(Context),value=view.owner===owner&&owner?view[input]:unknown()[input];
  const label=input==="microphone"?"マイクの取得状態":"PC音声の取得状態",status=inputLabel(value,input);
  if(indicator)return <span className="mini-input-indicator" data-input-kind={input} data-input-state={value.state} role="img" aria-label={`${label}：${status}`} title={status} />;
  return <span className="recording-input-status" data-input-kind={input} data-input-state={value.state} aria-label={label} title={status}>{showPrefix?"接続状態: ":""}{status}</span>;
}
