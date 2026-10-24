import {unpackNoticeReply} from "./archived-input-state";
import {invoke,isTauri} from "@tauri-apps/api/core";
import {listen} from "@tauri-apps/api/event";
import {useEffect,useRef,useState} from "react";
import {ArchivedInputController,type ArchivedAttempt,type ArchivedInputView} from "./archived-input-controller";
import {InputLevelClock} from "./input-level-state";
import type {InputKind} from "./recording-input-state";
import {IssueNotice,ToastNotice,toasts} from "./toast";
import {toastKey} from "./toast-state";

const lanes:InputKind[]=["microphone","system"];
const scope=(owner:string)=>`archived-input:${owner}`;
function OwnerNotices({view,locked,onOpen,onRendered,onClose}:{view:ArchivedInputView;locked:boolean;onOpen:(id:string)=>Promise<void>;onRendered:(owner:string,id:string,generation:number)=>void;onClose:(owner:string,input:InputKind)=>void}){
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
  const [views,setViews]=useState<ArchivedInputView[]>([]),[readError,setReadError]=useState(""),[listenerError,setListenerError]=useState(""),[reading,setReading]=useState(false),[readPending,setReadPending]=useState(false);
  const guard=useRef({paused,locked,onOpen,meetingIds}),refresh=useRef<()=>void>(()=>{}),actions=useRef({rendered:(_owner:string,_id:string,_generation:number)=>{},close:(_owner:string,_input:InputKind)=>{}});
  guard.current={paused,locked,onOpen,meetingIds};
  useEffect(()=>{
    if(!isTauri())return;
    let disposed=false,inFlight=false,epoch=0,wasPaused=guard.current.paused,next=0,pendingAt:number|null=null;
    const controller=new ArchivedInputController(crypto.randomUUID()),clock=new InputLevelClock(performance.now(),Date.now()),stops:(()=>void)[]=[];
    controller.pause(wasPaused);
    const now=()=>clock.read(performance.now(),Date.now());
    const pause=()=>{if(wasPaused!==guard.current.paused){wasPaused=guard.current.paused;epoch++;controller.pause(wasPaused);}return wasPaused;};
    const paint=()=>{
      if(disposed)return;pause();if(guard.current.meetingIds)controller.retainMeetings(new Set(guard.current.meetingIds));const at=now(),state=controller.paint(at);setViews(state.views);
      setReadPending(!wasPaused&&pendingAt!==null&&at-pendingAt>=1000);
      for(const request of state.attempts)void deliver(request);
    };
    const deliver=async(request:ArchivedAttempt)=>{
      const a=request.attempt;
      try{const value=await invoke("acknowledge_archived_recording_input_notice",{request:{meetingId:request.meetingId,sessionId:a.sessionId,noticeId:a.noticeId,recipientId:a.recipientId,generation:a.generation,action:a.action,receiveId:a.action==="receive"?a.receiveId:null}});
        if(disposed)return;pause();const reply=unpackNoticeReply(value);if(reply.paused){controller.defer(request);paint();return;}controller.complete(request,reply.value,now());paint();
      }catch{if(!disposed){controller.fail(request);paint();}}
    };
    const poll=async()=>{
      if(disposed||pause()||inFlight)return;
      const requestedAt=now();if(!Number.isFinite(requestedAt)){controller.failedRead();setReadError("時計の変更により通知の受付期限を確認できません。アプリを再起動して通知履歴を確認してください。");paint();return;}
      inFlight=true;pendingAt=requestedAt;setReading(true);const after=controller.after(),requestedEpoch=epoch;
      try{const page=await invoke("get_archived_recording_input_notices",{afterSessionId:after});
        if(disposed||pause()||epoch!==requestedEpoch)return;
        const reply=unpackNoticeReply(page);if(reply.paused){controller.defer();return;}
        controller.acceptPage(reply.value,after,requestedAt);setReadError("");
      }catch{if(!disposed&&!pause()&&epoch===requestedEpoch){controller.failedRead();setReadError("終了済み録音の通知履歴を確認できません。保存された音声の成否を示す診断ではありません。");}}
      finally{inFlight=false;pendingAt=null;if(!disposed){setReading(false);paint();
        // One page at a time; traverse promptly instead of refreshing only the
        // first 32. Yield a frame so actual rendering can confirm delivery.
        if(!guard.current.paused&&controller.after()!==null)next=window.setTimeout(()=>void poll(),50);
      }}
    };
    refresh.current=()=>{clearTimeout(next);void poll();};
    actions.current={rendered:(owner,id,generation)=>{if(disposed)return;pause();controller.rendered(owner,id,generation,now());paint();},close:(owner,input)=>{if(disposed)return;controller.dismiss(owner,input);paint();}};
    void poll();const painting=window.setInterval(paint,100),polling=window.setInterval(()=>void poll(),500);
    for(const event of ["session-changed","recordings-recovered","meetings-changed"]){
      void listen(event,()=>refresh.current()).then(stop=>{if(disposed)stop();else stops.push(stop);}).catch(()=>{if(!disposed)setListenerError("通知履歴の更新イベントを受信できません。定期読込みは継続しています。");});
    }
    return()=>{disposed=true;clearTimeout(next);clearInterval(painting);clearInterval(polling);stops.forEach(stop=>stop());refresh.current=()=>{};actions.current={rendered:()=>{},close:()=>{}};for(const type of ["read","listen"])toasts.resolve(toastKey("archived-input",type));};
  },[]);
  useEffect(()=>{refresh.current();},[paused]);
  const open=async(id:string)=>{if(!guard.current.locked)await guard.current.onOpen(id);};
  return <>
    {views.map(view=><OwnerNotices key={view.sessionId} view={view} locked={locked} onOpen={open} onRendered={(...args)=>actions.current.rendered(...args)} onClose={(...args)=>actions.current.close(...args)}/>)}
    <IssueNotice scope="archived-input" type="read" message={readError} label="終了済み録音の通知読込" action={{kind:"retry",label:"通知履歴を再読込",disabled:paused||reading,run:()=>refresh.current()}} />
    <IssueNotice scope="archived-input" type="listen" message={listenerError} label="終了済み録音の通知受信"/>
    <ToastNotice scope="archived-input" type="pending" level="info" message={readPending?"終了済み録音の通知履歴を確認中です。音声の取得・保存結果を示す状態ではありません。":""}/>
    {views.map(view=>{
      const diagnostics=[
        view.unavailable?"通知履歴を読めません":"",
        view.historyIssue==="missing"?"取得履歴がありません":view.historyIssue==="invalid"?"取得履歴を照合できません":"",
        view.deliveryUnavailable?"復元した通知の受付を確認できません":"",
        view.historySaveIssue?({failed:"取得履歴の書込みを確定できません",invalid:"取得履歴の保存を停止しました",unconfirmed:"停止時の取得履歴保存が未確定です",unavailable:"停止時の履歴保存診断を確認できません"}[view.historySaveIssue]):"",
        view.loss.microphone>0?`マイク通知の保持不足 ${view.loss.microphone}件`:"",
        view.loss.system>0?`PC音声通知の保持不足 ${view.loss.system}件`:"",
        lanes.some(input=>view.reception[input]==="unconfirmed")?"通知処理の結果確認待ち":""
      ].filter(Boolean).join(" / ");
      return <ToastNotice key={view.sessionId} scope={scope(view.sessionId)} type="diagnostic" level="warning" important historyOnly meetingId={view.meetingId}
        message={diagnostics?`終了した録音「${view.title}」：${diagnostics}。保存された音声で欠損範囲を確認してください。`:""}
        action={{kind:"open",label:"会議を開く",disabled:locked,run:()=>open(view.meetingId)}}/>;
    })}
  </>;
}
