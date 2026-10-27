import {unpackNoticeReply} from "./archived-input-state";
import {invoke,isTauri} from "@tauri-apps/api/core";
import {listen} from "@tauri-apps/api/event";
import {getCurrentWindow} from "@tauri-apps/api/window";
import {ArchivedInputScheduler} from "./archived-input-scheduler";
import {useEffect,useRef,useState} from "react";
import {ArchivedInputController,type ArchivedAttempt,type ArchivedInputView} from "./archived-input-controller";
import {InputLevelClock} from "./input-level-state";
import type {InputKind} from "./recording-input-state";
import {IssueNotice,ToastNotice,toasts} from "./toast";
import {toastKey} from "./toast-state";
import {archivedDiagnosticMessage} from "./input-diagnostic-copy";
import {InputDiagnosticLog} from "./input-diagnostic-log";

const lanes:InputKind[]=["microphone","system"];
const scope=(owner:string)=>`archived-input:${owner}`;
function OwnerNotices({view,locked,onOpen,onRendered,onClose}:{view:ArchivedInputView;locked:boolean;onOpen:(id:string)=>Promise<void>;onRendered:(owner:string,id:string,generation:number)=>void;onClose:(owner:string,input:InputKind)=>void}){
  useEffect(()=>{if(view.historySaveConfirmed)toasts.settleHistory(toastKey(view.sessionId,"journal-save"),`終了した録音「${view.title}」の接続記録が保存されていることを確認しました。`);},[view.sessionId,view.title,view.historySaveConfirmed]);
  // ToastNotice normally outlives a view. This bounded historical owner must
  // explicitly resolve its presentation on eviction/deletion/unmount instead.
  useEffect(()=>()=>{for(const input of [...lanes,"diagnostic"])toasts.resolve(toastKey(scope(view.sessionId),input));},[view.sessionId]);
  return <>{lanes.map(input=>{
    const n=view.notices.find(n=>n.input===input),label=input==="microphone"?"マイク":"PC音声";
    return <ToastNotice key={input} scope={scope(view.sessionId)} type={input}
      meetingId={view.meetingId}
      message={n?`終了した録音「${view.title}」：${label}の取得が${n.kind==="interrupted"?"中断していました":"復帰していました"}。保存された音声と欠損範囲を確認してください。`:""}
      level={n?.kind==="interrupted"?"warning":"info"} important={n?.kind==="interrupted"}
      eventId={n?`${n.id}:${n.delivery!.generation}`:undefined}
      onRendered={n?()=>onRendered(view.sessionId,n.id,n.delivery!.generation):undefined}
      onClose={()=>onClose(view.sessionId,input)} action={n?{kind:"open",label:"会議を開く",disabled:locked,run:()=>onOpen(view.meetingId)}:undefined}/>;
  })}</>;
}

/** Closed recordings only: neither a current connection badge nor replay. */
export function ArchivedInputNotices({paused,locked,onOpen,meetingIds}:{meetingIds?:string[];paused:boolean;locked:boolean;onOpen:(id:string)=>Promise<void>}){
  const [views,setViews]=useState<ArchivedInputView[]>([]),[readError,setReadError]=useState(""),[listenerError,setListenerError]=useState(""),[windowError,setWindowError]=useState(""),[reading,setReading]=useState(false),[readConfirmed,setReadConfirmed]=useState(false);
  const guard=useRef({paused,locked,onOpen,meetingIds}),refresh=useRef<()=>void>(()=>{}),syncProps=useRef<()=>void>(()=>{}),actions=useRef({rendered:(_owner:string,_id:string,_generation:number)=>{},close:(_owner:string,_input:InputKind)=>{}});
  guard.current={paused,locked,onOpen,meetingIds};
  useEffect(()=>{
    if(!isTauri())return;
    let disposed=false,nativeVisible=false,visibilityEpoch=0,painting=0,visibilityRetry=0,visibilityFailures=0;
    const diagnosticLog=new InputDiagnosticLog(),controller=new ArchivedInputController(crypto.randomUUID(),(owner,error)=>diagnosticLog.report("archived-validation",owner,error)),clock=new InputLevelClock(performance.now(),Date.now()),stops:(()=>void)[]=[];
    controller.pause(true);
    let knownMeetings=new Set(guard.current.meetingIds??[]),operationEpoch=0,wasPaused=guard.current.paused;
    const now=()=>clock.read(performance.now(),Date.now());
    const blocked=()=>guard.current.paused||document.hidden||!nativeVisible;
    const paint=()=>{
      if(disposed)return;
      clearTimeout(painting);scheduler.setBlocked(blocked());
      // A newly closed owner can precede the meeting-list refresh. Only an
      // actual removal from a prior UI snapshot is deletion evidence here.
      if(guard.current.meetingIds){
        const next=new Set(guard.current.meetingIds);
        controller.removeMeetings(new Set([...knownMeetings].filter(id=>!next.has(id))));knownMeetings=next;
      }
      const state=controller.paint(now());setViews(state.views);
      for(const request of state.attempts)void deliver(request);
      scheduler.sync();
      if(!blocked()&&controller.needsPaint())painting=window.setTimeout(paint,100);
    };
    const scheduler=new ArchivedInputScheduler(controller,{now,start:(run,delay)=>window.setTimeout(run,delay),stop:timer=>clearTimeout(timer as number)},
      request=>request.kind==="page"?invoke("get_archived_recording_input_notices",{afterSessionId:request.after}):invoke("get_archived_recording_input_notice_rows",{targets:request.targets}),
      (reading,error)=>{if(!disposed){setReading(reading);setReadError(error);paint();}},
      (request,error)=>{setReadConfirmed(error===null);diagnosticLog.report(request.kind==="page"?"archived-page":"archived-rows",null,error,request);});
    const deliver=async(request:ArchivedAttempt)=>{
      if(disposed||blocked())return;
      const a=request.attempt,epoch=operationEpoch;
      try{
        const value=await invoke("acknowledge_archived_recording_input_notice",{request:{meetingId:request.meetingId,sessionId:a.sessionId,noticeId:a.noticeId,recipientId:a.recipientId,generation:a.generation,action:a.action,receiveId:a.action==="receive"?a.receiveId:null}});
        if(disposed)return;
        const reply=unpackNoticeReply(value);
        if(reply.paused){controller.defer(request);paint();return;}
        // Reconcile the original RPC slot, but never resurrect paused display.
        if(blocked()||epoch!==operationEpoch)controller.deferTargets([{meetingId:request.meetingId,sessionId:a.sessionId}]);
        controller.complete(request,reply.value,now());paint();
      }catch{if(!disposed){controller.fail(request);paint();}}
    };
    refresh.current=()=>scheduler.invalidate(true);
    syncProps.current=()=>{if(wasPaused!==guard.current.paused){operationEpoch++;wasPaused=guard.current.paused;}scheduler.setBlocked(blocked());paint();};
    actions.current={rendered:(owner,id,generation)=>{if(disposed||blocked())return;controller.rendered(owner,id,generation,now());paint();},close:(owner,input)=>{if(disposed)return;controller.dismiss(owner,input);paint();}};
    const nativeWindow=getCurrentWindow();
    const checkVisibility=async(wake=false)=>{
      const epoch=++visibilityEpoch;clearTimeout(visibilityRetry);
      try{
        const [visible,minimized]=await Promise.all([nativeWindow.isVisible(),nativeWindow.isMinimized()]);
        if(disposed||epoch!==visibilityEpoch)return;
        if(typeof visible!=="boolean"||typeof minimized!=="boolean")throw Error("画面状態の応答が不正です");
        if(nativeVisible!==(visible&&!minimized))operationEpoch++;
        nativeVisible=visible&&!minimized;visibilityFailures=0;setWindowError("");diagnosticLog.report("archived-window",null,null);scheduler.setBlocked(blocked());
        if(wake&&!blocked())scheduler.invalidate();paint();
      }catch(error){
        if(disposed||epoch!==visibilityEpoch)return;
        diagnosticLog.report("archived-window",null,error);
        nativeVisible=false;scheduler.setBlocked(true);paint();
        setWindowError("通知を表示できる画面か確認できないため、接続記録の確認を一時的に待っています。自動で再確認します。");
        visibilityRetry=window.setTimeout(()=>void checkVisibility(),Math.min(60_000,1000*2**Math.min(6,visibilityFailures++)));
      }
    };
    const subscribe=async(promise:Promise<()=>void>)=>{
      try{const stop=await promise;if(disposed)stop();else stops.push(stop);}
      catch(error){if(!disposed){diagnosticLog.report("archived-subscribe",null,error);setListenerError("接続記録の更新通知を受け取れません。60秒ごとに自動で確認します。録音ファイルの保存失敗を示す通知ではありません。");}}
    };
    const focus=()=>void checkVisibility(true),visibility=()=>{scheduler.setBlocked(blocked());paint();void checkVisibility(!document.hidden);};
    window.addEventListener("focus",focus);document.addEventListener("visibilitychange",visibility);
    // Subscribe first, then read a snapshot to close the startup event gap.
    void Promise.all([
      ...["session-changed","recordings-recovered","meetings-changed"].map(event=>subscribe(listen(event,()=>scheduler.invalidate()))),
      subscribe(nativeWindow.onResized(()=>void checkVisibility())),
      subscribe(nativeWindow.onFocusChanged(event=>void checkVisibility(event.payload))),
    ]).then(async()=>{if(!disposed){await checkVisibility();if(!disposed)scheduler.start(blocked());}});
    return()=>{
      disposed=true;visibilityEpoch++;scheduler.dispose();clearTimeout(painting);clearTimeout(visibilityRetry);stops.forEach(stop=>stop());
      window.removeEventListener("focus",focus);document.removeEventListener("visibilitychange",visibility);
      refresh.current=()=>{};syncProps.current=()=>{};actions.current={rendered:()=>{},close:()=>{}};
      for(const type of ["read","listen","pending"])toasts.resolve(toastKey("archived-input",type));
    };
  },[]);
  useEffect(()=>{syncProps.current();},[paused,meetingIds]);
  const open=async(id:string)=>{if(!guard.current.locked)await guard.current.onOpen(id);};
  return <>
    {views.map(view=><OwnerNotices key={view.sessionId} view={view} locked={locked} onOpen={open} onRendered={(...args)=>actions.current.rendered(...args)} onClose={(...args)=>actions.current.close(...args)}/>)}
    <ToastNotice scope="archived-input" type="read" message={readError} level="warning" resolutionMessage={readConfirmed?"過去の録音の接続記録を再確認できました。読取りの問題は解消しています。":undefined} action={{kind:"retry",label:"もう一度確認",disabled:paused||reading,run:()=>refresh.current()}} />
    <IssueNotice scope="archived-input" type="listen" message={listenerError||windowError} label="過去の録音の接続記録"/>
    {views.map(view=>{
      return <ToastNotice key={view.sessionId} scope={scope(view.sessionId)} type="diagnostic" level="warning" important historyOnly meetingId={view.meetingId}
        message={archivedDiagnosticMessage(view)} resolutionMessage={view.checked?"接続記録と通知を読み取れました。確認できなかった状態は解消しています。":undefined}
        action={{kind:"open",label:"会議を開く",disabled:locked,run:()=>open(view.meetingId)}}/>;
    })}
  </>;
}
