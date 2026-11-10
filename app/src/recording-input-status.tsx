import {invoke,isTauri} from "./ipc.ts";
import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from "react";
import {useRecordingSession} from "./session";
import {InputLevelClock} from "./input-level-state";
import {RecordingInputSession,inputLabel,type InputKind,type VisibleInput,type InputNotice} from "./recording-input-state";
import {RecordingInputQueue,type QueueAttempt,type NoticeReception} from "./recording-input-queue";
import {ToastNotice} from "./toast";
import {InputJournalSession,InputJournalFailures} from "./input-journal-state";
import "./recording-input-status.css";
import {InputDiagnosticLog} from "./input-diagnostic-log";
type View={owner:string|null;microphone:VisibleInput;system:VisibleInput;notices:InputNotice[];reception:Record<InputKind,NoticeReception>;dismiss:(input:InputKind)=>void;rendered:(id:string,generation:number)=>void};
const unknown=()=>({microphone:{state:"unknown"} as VisibleInput,system:{state:"unknown"} as VisibleInput,notices:[] as InputNotice[]});
const empty=()=>({owner:null,...unknown(),reception:{microphone:"none",system:"none"} as Record<InputKind,NoticeReception>,dismiss:(_input:InputKind)=>{},rendered:(_id:string,_generation:number)=>{}});
const Context=createContext<View>(empty());
export function RecordingInputProvider({children,windowLabel}:{children:ReactNode;windowLabel:"main"|"recorder"}){
  const recording=useRecordingSession(),owner=recording.kind==="ready"&&["preparing","recording"].includes(recording.snapshot.phase)?recording.snapshot.sessionId:null;
  const phaseRef=useRef(recording.kind==="ready"?recording.snapshot.phase:"idle");phaseRef.current=recording.kind==="ready"?recording.snapshot.phase:"idle";
  const [view,setView]=useState<View>(empty());
  useEffect(()=>{
    let disposed=false,inFlight=false,failed=true;const session=new RecordingInputSession(),clock=new InputLevelClock(performance.now(),Date.now());
    setView({...empty(),owner});try{session.reset(owner);}catch{return;}if(!owner||!isTauri())return;
    // A failed session bridge disposes this controller even if its WebView
    // survives. Reopening it must not impersonate a previously accepted receiver.
    const queue=new RecordingInputQueue(owner,crypto.randomUUID(),windowLabel);
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
      // The mini panel presents telemetry only. Do not receive or acknowledge
      // events it cannot show: unreceived events can still reach the main
      // window when it is opened, with native ownership/lease checks intact.
      if(windowLabel==="recorder"){setView({...empty(),owner,...raw,notices:[]});return;}
      const attempts=queue.observe(raw.notices,at);setView({owner,...raw,...queue.read(at),dismiss:input=>{if(!disposed){queue.dismiss(input);paint();}},rendered:(id,generation)=>{if(!disposed){queue.rendered(id,generation,now());paint();}}});for(const attempt of attempts)void deliver(attempt);
    };
    const deliver=async(attempt:QueueAttempt)=>{try{
      const command={receive:"receive_recording_input_notice",render:"render_recording_input_notice",close:"close_recording_input_notice"}[attempt.action];
      const value=await invoke(command,{sessionId:attempt.sessionId,noticeId:attempt.noticeId,recipientId:attempt.recipientId,generation:attempt.generation,...(attempt.action==="receive"?{receiveId:attempt.receiveId}:{})});
      if(disposed)return;queue.complete(attempt,value,now());paint();
    }catch{if(!disposed){queue.fail(attempt);paint();}}};
    const poll=async()=>{if(disposed||inFlight)return;inFlight=true;const requestedAt=now();
      try{const report=await invoke("get_recording_input_status",{sessionId:owner});if(!disposed){session.accept(report,requestedAt);failed=false;}}
      catch{if(!disposed)failed=true;}finally{inFlight=false;paint();}
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
  const reception=view.owner===owner&&owner?view.reception[input]:"none",waiting=reception==="unconfirmed";
  const label=input==="microphone"?"マイクの取得状態":"PC音声の取得状態",status=inputLabel(value,input);
  if(indicator)return <span className="mini-input-indicator" data-input-kind={input} data-input-state={value.state} role="img" aria-label={`${label}：${status}`} title={status} />;
  return <span className="recording-input-status" data-input-kind={input} data-input-state={value.state} data-notice-reception={reception} aria-label={label} title={status+(waiting?"。通知の処理結果を確認中です。取得や保存の成功を示す状態ではありません。":"")}>{showPrefix?"接続状態: ":""}{status}{waiting?" / 通知確認待ち":""}</span>;
}
const message=(notice:InputNotice)=>notice.input==="microphone"?(notice.kind==="interrupted"?(notice.delivery.status.reason==="device_changed"?"マイクを切り替えています。切替中の欠損範囲は保存後に確認します。":"マイクの取得が中断しました。選択した機器と権限を確認してください。"):(notice.delivery.status.sourceEpoch>1?"マイクの取得が復帰しました。欠損範囲は保存後に確認します。":"マイクの取得が開始しました。")):
  (notice.kind==="interrupted"?"PC音声の取得が中断しました。取得対象と権限を確認してください。":"同じ対象のPC音声取得が復帰しました。欠損範囲は保存後に確認します。");
export function RecordingInputNotice({windowLabel}:{windowLabel:"main"}){
  const view=useContext(Context),notice=(input:InputKind)=>view.notices.find(n=>n.input===input&&n.target===windowLabel)??null;
  const microphone=notice("microphone"),system=notice("system"),closeMicrophone=()=>view.dismiss("microphone"),closeSystem=()=>view.dismiss("system");
  const entries:[InputKind,InputNotice|null,()=>void][]=[["microphone",microphone,closeMicrophone],["system",system,closeSystem]];
  return <>{entries.map(([input,n,close])=>{
    const expectedSwitch=n?.input==="microphone"&&n.kind==="interrupted"&&n.delivery.status.reason==="device_changed";
    const interrupted=n?.kind==="interrupted"&&!expectedSwitch;
    return <ToastNotice key={input} scope={view.owner??"recording-input"} type={`${input}-status`}
      message={n?`録音中の履歴：${message(n)}`:""} level={interrupted?"warning":"info"} important={interrupted}
      eventId={n?`${n.id}:${n.delivery.generation}`:undefined} onClose={close} onRendered={n?()=>view.rendered(n.id,n.delivery.generation):undefined}/>;
  })}</>;
}
