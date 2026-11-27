import {invoke,isTauri} from "./ipc.ts";
import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from "react";
import {useRecordingSession} from "./session";
import {RecordingInputSession,checkedInputStatus,inputLabel,type InputKind,type VisibleInput} from "./recording-input-state";
import {InputJournalSession,InputJournalFailures,checkedJournalReport,type JournalView} from "./input-journal-state";
import "./recording-input-status.css";
import {InputDiagnosticLog,type InputReadStage} from "./input-diagnostic-log";
import {RecoverableRead,sharedClock,type RecoverableAnomaly} from "./recoverable-read.ts";
import {advanceSharedRecordingClock} from "./recording-input-advance.ts";

type View={owner:string|null;microphone:VisibleInput;system:VisibleInput};
const unknown=()=>({microphone:{state:"unknown"} as VisibleInput,system:{state:"unknown"} as VisibleInput});
const empty=():View=>({owner:null,...unknown()});
const Context=createContext<View>(empty());

class JournalSessionAdapter{
  private session:InputJournalSession;
  constructor(owner:string){this.session=new InputJournalSession(owner);}
  reset(owner:string|null){if(owner)this.session=new InputJournalSession(owner);}
  accept(value:unknown,at:number){this.session.accept(value,at);return true;}
  read(at:number):JournalView{return this.session.read(at);}
  fail(){this.session.fail();}
}

export function RecordingInputProvider({children,windowLabel}:{children:ReactNode;windowLabel:"main"|"recorder"}){
  const recording=useRecordingSession(),owner=recording.kind==="ready"&&["preparing","recording"].includes(recording.snapshot.phase)?recording.snapshot.sessionId:null;
  const phaseRef=useRef(recording.kind==="ready"?recording.snapshot.phase:"idle");phaseRef.current=recording.kind==="ready"?recording.snapshot.phase:"idle";
  const [view,setView]=useState<View>(empty());
  useEffect(()=>{
    let disposed=false;
    setView({...empty(),owner});
    if(!owner||!isTauri())return;
    const diagnosticLog=new InputDiagnosticLog();
    diagnosticLog.bindWindow(windowLabel);
    const mono=()=>performance.now();
    const wall=()=>Date.now();
    const clock=sharedClock(mono(),wall());
    const statusSession=new RecordingInputSession();
    let journalSession:JournalSessionAdapter;
    try{
      statusSession.reset(owner);
      journalSession=new JournalSessionAdapter(owner);
    }catch{
      return()=>{diagnosticLog.dispose();};
    }
    const journalFailures=new InputJournalFailures();
    const reportAnomaly=(stage:InputReadStage)=>(kind:RecoverableAnomaly,detail:Record<string,unknown>)=>diagnosticLog.reportAnomaly(stage,owner,kind,detail);
    const statusPoll=new RecoverableRead(owner,statusSession,clock,{
      mono,wall,
      invoke:()=>invoke("get_recording_input_status",{sessionId:owner}),
      revision:value=>{try{const r=checkedInputStatus(value,owner);return {revision:r.revision,reportAgeMs:r.reportAgeMs};}catch{return null;}},
      onAnomaly:reportAnomaly("live-status"),
      // Each changed read error and its recovery are logged at once, as before.
      onRead:error=>diagnosticLog.report("live-status",owner,error),
      onAdopted:()=>{if(!disposed)paint();},
    });
    const journalPoll=new RecoverableRead<JournalView>(owner,journalSession,clock,{
      mono,wall,
      invoke:()=>invoke("get_recording_input_history_status",{sessionId:owner}),
      revision:value=>{try{const r=checkedJournalReport(value,owner);return {revision:r.revision,reportAgeMs:r.reportAgeMs};}catch{return null;}},
      onAnomaly:reportAnomaly("live-journal"),
      onRead:error=>diagnosticLog.report("live-journal",owner,error,error===null?{}:{command:"get_recording_input_history_status"}),
      onAdopted:()=>{if(journalPoll.failed)journalSession.fail();if(!disposed)paint();},
    });
    const logJournal=(at:number)=>{
      const journalView=journalPoll.failed?{state:"unknown" as const,problem:false}:journalPoll.read();
      const failure=journalFailures.read(journalView,phaseRef.current,at);
      if(failure){
        const message=failure==="failed"
          ?"マイク・PC音声の中断・再開履歴を保存できませんでした。"
          :"マイク・PC音声の中断・再開履歴に不整合が見つかり、保存を停止しました。";
        diagnosticLog.report("live-journal-notice",owner,message,{state:failure});
      }else if(journalView.state==="confirmed")diagnosticLog.report("live-journal-notice",owner,null);
    };
    const paint=()=>{
      if(disposed)return;
      const at=clock.read(mono(),wall()),read=statusPoll.failed?unknown():statusPoll.read() as {microphone:VisibleInput;system:VisibleInput};
      logJournal(at);
      setView({owner,microphone:read.microphone,system:read.system});
    };
    // Freshness, clock reversal and hung reads are checked every 100 ms; the
    // native status files are still read at the existing 500 ms cadence.
    const tick=()=>{advanceSharedRecordingClock(clock,statusPoll,journalPoll,mono,wall);paint();};
    const poll=()=>{statusPoll.poll();journalPoll.poll();};
    tick();poll();
    const painting=window.setInterval(tick,100),polling=window.setInterval(poll,500);
    return()=>{disposed=true;window.clearInterval(painting);window.clearInterval(polling);statusPoll.dispose();journalPoll.dispose();diagnosticLog.dispose();};
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
