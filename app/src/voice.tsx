import { ErrorNotice } from "./toast";
import { InfoPopover } from "./info-popover";
import {invoke,isTauri} from "@tauri-apps/api/core";
import {listen} from "@tauri-apps/api/event";
import {IconButton} from "./action-icon";
import {useTranscriptAudio} from "./transcript-audio";
import {stopAllPlayback} from "./playback-barrier";
import {useEffect,useRef,useState} from "react";
import {CommonModal} from "./common-modal";
import {flushPendingEdits} from "./draft-save";
import {voiceMatchLabel,voiceStartRequest,voiceWindowKey,initialVoiceSelection,type VoiceContext,type VoicePerson,type VoiceWindow} from "./voice-state";
import type {Speaker} from "./transcript-state";
import type {AnalysisJob} from "./analysis-state";
import "./voice.css";
import {OperationProgressBar} from "./operation-progress";
import {jobProgress} from "./operation-progress-state";

type VoiceJob=AnalysisJob&{operationId:string};
export function VoicePanel({meetingId,speakers,revision,locked,beforeOperation,onChanged,onPlay}:{meetingId:string;speakers:Speaker[];revision:string;locked:boolean;beforeOperation:()=>Promise<void>;onChanged:()=>Promise<void>;onPlay:(window:VoiceWindow)=>Promise<void>}){
  const [open,setOpen]=useState(false),[context,setContext]=useState<VoiceContext|null>(null),[error,setError]=useState(""),[busy,setBusy]=useState(false),[notice,setNotice]=useState("");
  const [enrolling,setEnrolling]=useState<string|null>(null),[name,setName]=useState(""),[person,setPerson]=useState("");
  const [jobs,setJobs]=useState<VoiceJob[]>([]);
  const [selected,setSelected]=useState<string[]>([]),[page,setPage]=useState(0),[needsReview,setNeedsReview]=useState(false),[refreshing,setRefreshing]=useState(false);
  const selection=useRef(selected);selection.current=selected;
  const enrollingRef=useRef(enrolling);enrollingRef.current=enrolling;
  const refreshSequence=useRef(0);
  const pending=useRef<ReturnType<typeof voiceStartRequest>|null>(null),active=useRef(true),inFlight=useRef(false);
  const latest=useRef(onChanged);latest.current=onChanged;
  const opener=useRef<HTMLElement|null>(null);
  const refresh=async()=>{
    const sequence=++refreshSequence.current;setRefreshing(true);
    try {
      const [next,all]=await Promise.all([invoke<VoiceContext>("get_voice_context",{meetingId}),invoke<VoiceJob[]>("get_analysis_jobs",{meetingId})]);
      if(active.current&&sequence===refreshSequence.current){
        if(!pending.current)setContext(next);
        setJobs(all.filter(j=>j.kind==="voice_registration"||j.kind==="voice_matching"||j.kind==="speaker_completion"));
        const valid=new Set((next.registrationCandidates??[]).filter(w=>w.speakerId===enrollingRef.current).map(voiceWindowKey));
        if(!pending.current&&enrollingRef.current&&selection.current.some(key=>!valid.has(key))){setSelected(keys=>keys.filter(key=>valid.has(key)));setNeedsReview(true);setPage(0);}
      }
      return next;
    } catch(cause) {
      if(active.current&&sequence===refreshSequence.current){if(!pending.current)setContext(null);setError(String(cause));}
      throw cause;
    } finally {if(active.current&&sequence===refreshSequence.current)setRefreshing(false);}
  };
  useEffect(()=>{active.current=true;return()=>{active.current=false;};},[]);
  useEffect(()=>{
    if(!open||!isTauri())return;
    let stopped=false,timer:ReturnType<typeof setTimeout>;
    const poll=async()=>{try{const all=await invoke<VoiceJob[]>("get_analysis_jobs",{meetingId});if(!stopped)setJobs(all.filter(j=>j.kind==="voice_registration"||j.kind==="voice_matching"||j.kind==="speaker_completion"));}catch(cause){if(!stopped)setError(String(cause));}if(!stopped)timer=setTimeout(()=>void poll(),1500);};
    void poll();return()=>{stopped=true;clearTimeout(timer);};
  },[open,meetingId]);
  useEffect(()=>{
    if(!open||!isTauri())return;
    void refresh().catch(()=>{});
  },[open,meetingId,revision]);
  const completed=useRef(new Set<string>());
  useEffect(()=>{
    const fresh=jobs.filter(j=>j.state==="succeeded"&&!completed.current.has(j.id));
    if(!fresh.length)return;
    fresh.forEach(j=>completed.current.add(j.id));
    void latest.current().then(refresh).then(()=>{if(active.current)setNotice("");}).catch(c=>{if(active.current)setError(String(c));});
  },[jobs]);
  const run=async(action:()=>Promise<void>)=>{if(inFlight.current||locked)return;inFlight.current=true;setBusy(true);setError("");setNotice("");try{await action();}catch(c){if(active.current)setError(String(c));}finally{inFlight.current=false;if(active.current)setBusy(false);}};
  const prepare=async()=>{await beforeOperation();return refresh();};
  const start=async(speakerId:string|null)=>{
    if(speakerId&&needsReview&&!pending.current)throw Error("登録候補が変更されています。区間を確認してください。");
    const c=pending.current?context!:await prepare();
    const request=pending.current??voiceStartRequest(meetingId,c,speakerId,person||null,name,crypto.randomUUID(),speakerId?selected:undefined);pending.current=request;
    const response=await invoke<{state:"accepted";job:VoiceJob}|{state:"rejected";message:string}>("start_voice_job",{request});
    if(response.state==="rejected") {pending.current=null;await refresh();throw Error(response.message);}
    const job=response.job;
    if(job.operationId!==request.operationId||job.kind!==(request.speakerId?"voice_registration":"voice_matching"))throw Error("声の処理の受付結果が一致しません。再送して確認してください。");
    pending.current=null;setEnrolling(null);setNotice("声の処理を受け付けました。");await refresh();
  };
  const disabled=locked||busy||refreshing;
  const candidates=(context?.registrationCandidates??[]).filter(w=>w.speakerId===enrolling);
  const pageCount=Math.max(1,Math.ceil(candidates.length/20));
  const currentPage=Math.min(page,pageCount-1);
  const time=(us:number)=>{const ms=Math.floor(us/1000);return `${Math.floor(ms/60000)}:${(ms%60000/1000).toFixed(3).padStart(6,"0")}`;};
  const running=jobs.some(j=>["queued","running","pausing"].includes(j.state));
  return <details className="voice-panel" onToggle={event=>setOpen(event.currentTarget.open)}>
    <summary>声の登録・照合</summary>
    {open&&<>
      <div className="heading-with-info"><h3>声の登録・照合</h3><InfoPopover label="声の登録・照合"><p>会議の声を人物として登録すると、以降の解析で照合します。名前の変更だけでは声を登録しません。</p><p>実音声で評価済みの判定条件がないモデルでは、名前を候補として表示します。</p></InfoPopover></div>
      {context&&!context.supported?<p>声の登録・照合は現在macOSのみ対応しています。</p>:<>
        <button className="small-secondary" disabled={disabled||running||!context?.people.some(p=>p.samples.length)} onClick={()=>void run(()=>start(null))}>登録した声と照合</button>
        {speakers.map(s=>{
          const match=context?.matches.find(m=>m.speakerId===s.id);
          return <div className="voice-speaker" key={s.id}>
            <strong>{s.displayName||s.label}</strong>
            <button className="small-secondary" aria-describedby={`voice-reason-${s.id}`} disabled={disabled||running||!!pending.current||!context?.registrationCandidates?.some(w=>w.speakerId===s.id)} onClick={event=>{opener.current=event.currentTarget;void run(async()=>{const fresh=await prepare();setPerson("");setName(s.displayName||"");setSelected(initialVoiceSelection(fresh.registrationCandidates.filter(w=>w.speakerId===s.id)));setPage(0);setNeedsReview(false);setEnrolling(s.id);});}}>声を登録</button>
            <small id={`voice-reason-${s.id}`} className="voice-registration-reason">{running?"声の処理中です。完了後に登録できます。":pending.current?"受付結果を確認してください。":refreshing?"登録候補を確認中です。":!context?"登録候補を取得できません。声の登録・照合を開き直してください。":!context.registrationCandidates?.some(w=>w.speakerId===s.id)?"登録できる1秒以上の単独音声がありません。":locked?"編集中の操作を完了し、保存状態を確認してください。":busy?"操作の完了をお待ちください。":""}</small>
            {match&&<><span>{voiceMatchLabel(match.state)}{match.state!=="dismissed"?`：${match.nameSnapshot}`:""}</span>
              {match.state!=="dismissed"&&<><select aria-label={`${s.displayName||s.label}に対応する人物`} disabled={disabled} value={match.personId??""} onChange={event=>{const id=event.target.value;setContext(c=>c&&({...c,matches:c.matches.map(m=>m.id===match.id?{...m,personId:id}:m)}));}}><option value="">人物を選択</option>{context?.people.map(p=><option key={p.id} value={p.id}>{p.name}（{p.id.slice(0,6)}）</option>)}</select>
              <button className="small-secondary" disabled={disabled||!match.personId} onClick={()=>void run(async()=>{await beforeOperation();const fresh=await refresh();await invoke("decide_voice_match",{meetingId,analysisId:fresh.analysisId,speakerId:s.id,personId:match.personId,expectedWorkingHash:fresh.workingHash});await onChanged();await refresh();})}>名前を適用</button>
              <button className="small-secondary" disabled={disabled} onClick={()=>void run(async()=>{await beforeOperation();const fresh=await refresh();await invoke("decide_voice_match",{meetingId,analysisId:fresh.analysisId,speakerId:s.id,personId:null,expectedWorkingHash:fresh.workingHash});await onChanged();await refresh();})}>対応を解除</button></>}
            </>}
          </div>;
        })}
      </>}
      {jobs.filter(job=>job.state!=="succeeded").slice(0,8).map(job=><div className="voice-job" key={job.id}><span>{job.kind==="speaker_completion"?"話者補完":job.kind==="voice_registration"?"声の登録":"声の照合"}：{({queued:"待機中",running:"処理中",pausing:"停止中",paused:"一時停止",interrupted:"中断",succeeded:"完了",failed:"失敗",cancelled:"取消済み"} as Record<string,string>)[job.state]??job.state}</span>
        {job.kind==="speaker_completion"&&jobProgress(job, "", meetingId)&&<OperationProgressBar progress={jobProgress(job, "", meetingId)!}/>}
        {["queued","running","paused","interrupted"].includes(job.state)&&<button className="small-secondary" disabled={disabled} onClick={()=>void run(async()=>{await invoke("cancel_analysis_job",{jobId:job.id});await refresh();})}>取り消す</button>}
        {["paused","interrupted"].includes(job.state)&&<button className="small-secondary" disabled={disabled} onClick={()=>void run(async()=>{await invoke("resume_analysis_job",{jobId:job.id});await refresh();})}>再開</button>}
        {["failed","cancelled"].includes(job.state)&&<button className="small-secondary" disabled={disabled} onClick={()=>void run(async()=>{await beforeOperation();await invoke("retry_voice_job",{jobId:job.id,operationId:crypto.randomUUID()});await refresh();})}>同じ条件で再試行</button>}
      </div>)}
      {notice&&<p role="status">{notice}</p>}<ErrorNotice scope="voice" type="VoicePanel-operation" active={!!(error)}>{error}</ErrorNotice>
      {pending.current&&<button className="small-secondary" disabled={disabled} onClick={()=>void run(()=>start(pending.current!.speakerId))}>同じ操作を再送して確認</button>}
      <CommonModal className="voice-dialog" nested open={!!enrolling} blocked={disabled} onDismiss={()=>setEnrolling(null)} restoreTarget={()=>opener.current} aria-label="声を人物として登録">
        <div className="heading-with-info"><h2>声を人物として登録</h2><InfoPopover label="声を人物として登録"><p>現在の話者に割り当てた発言から、登録する音声を1〜2区間選んでください。</p><p>1秒以上の単独音声を候補にしています。30秒を超える区間は中央の30秒を使います。</p></InfoPopover></div>
        <label>登録先<select value={person} disabled={disabled||!!pending.current} onChange={e=>setPerson(e.target.value)}><option value="">新しい人物</option>{context?.people.map(p=><option value={p.id} key={p.id}>{p.name}（{p.id.slice(0,6)}）</option>)}</select></label>
        {!person&&<label>人物名<input maxLength={200} value={name} disabled={disabled||!!pending.current} onChange={e=>setName(e.target.value)} /></label>}
        <p>選んだ区間を使い、この会議の話者にも人物名を適用します。元の会議・音声を削除しても、登録した声は残ります。</p>
        <p role="status">{selected.length} / 2区間を選択 · 候補{candidates.length}件</p>
        {needsReview&&<div role="alert"><p>登録候補が変更されました。区間を選び直すか、残った選択を確認してください。</p><button className="small-secondary" disabled={disabled||!!pending.current||!selected.length} onClick={()=>setNeedsReview(false)}>選択区間を確認しました</button></div>}
        {!context?<p>登録候補を取得できません。画面を開き直して確認してください。</p>:!candidates.length&&<p>登録できる1秒以上の単独音声がありません。</p>}
        <div className="voice-enrollment-windows">{candidates.slice(currentPage*20,(currentPage+1)*20).map((w,index)=>{
          const key=voiceWindowKey(w),checked=selected.includes(key),number=currentPage*20+index+1;
          return <div className="voice-enrollment-window" key={key}>
            <label><input type="checkbox" checked={checked} disabled={disabled||!!pending.current||(!checked&&selected.length>=2)} onChange={()=>{setSelected(keys=>checked?keys.filter(k=>k!==key):[...keys,key]);setNeedsReview(false);}} /><span>候補{number}：{time(w.startUs)}〜{time(w.endUs)}（{((w.endUs-w.startUs)/1e6).toFixed(3)}秒）</span></label>
            <p className="voice-candidate-text">{w.text}</p>
            <button className="small-secondary" disabled={disabled} aria-label={`候補${number}の登録区間を再生して確認`} onClick={()=>void run(()=>onPlay(w))}>登録区間を再生して確認</button>
          </div>;
        })}</div>
        {pageCount>1&&<nav className="voice-candidate-pages" aria-label="登録候補のページ"><IconButton label="前の20件" icon="previous" className="small-secondary" disabled={disabled||currentPage===0} onClick={()=>setPage(currentPage-1)} /><span>{currentPage+1} / {pageCount}</span><IconButton label="次の20件" icon="next" className="small-secondary" disabled={disabled||currentPage===pageCount-1} onClick={()=>setPage(currentPage+1)} /></nav>}
        {running&&!pending.current&&<p role="status">声の処理中です。完了後に登録できます。</p>}
        <button disabled={disabled||(!pending.current&&(running||!selected.length||needsReview||(!person&&!name.trim())))} onClick={()=>void run(()=>start(enrolling))}>{pending.current?"同じ登録を再送して確認":"声を登録"}</button>
        <ErrorNotice scope="voice" type="VoicePanel-operation" active={!!(error)}>{error}</ErrorNotice>
      </CommonModal>
    </>}
  </details>;
}

export function VoicePeopleSettings({locked=false,standalone=false}:{locked?:boolean;standalone?:boolean}){
  const [open,setOpen]=useState(standalone),[people,setPeople]=useState<VoicePerson[]>([]),[error,setError]=useState(""),[busy,setBusy]=useState(false),[loaded,setLoaded]=useState(false),[editing,setEditing]=useState<VoicePerson|null>(null),[name,setName]=useState(""),[deleting,setDeleting]=useState<VoicePerson|null>(null);
  const opener=useRef<HTMLElement|null>(null),inFlight=useRef(false),active=useRef(true);
  const audio=useTranscriptAudio("voice-library",locked,{inline:true,scope:"voice"});
  useEffect(()=>{active.current=true;return()=>{active.current=false;};},[]);
  const load=async()=>{const next=await invoke<VoicePerson[]>("get_voice_people");if(active.current){if(audio.selected&&!next.some(p=>p.samples.some(s=>s.id===audio.selected&&s.sourceAvailable)))audio.clear();setPeople(next);setLoaded(true);}};
  const run=async(action:()=>Promise<void>)=>{if(locked||inFlight.current)return;inFlight.current=true;setBusy(true);setError("");try{await flushPendingEdits();await action();await load();}catch(c){if(active.current)setError(String(c));}finally{inFlight.current=false;if(active.current)setBusy(false);}};
  const refresh=useRef(()=>run(async()=>{}));refresh.current=()=>run(async()=>{});
  useEffect(()=>{
    if(!open||locked)return;
    if(!isTauri()){setError("登録した声はデスクトップアプリで確認できます。");return;}
    void refresh.current();
    const subscription=listen("voice-identity-changed",()=>void refresh.current());
    return()=>{void subscription.then(unlisten=>unlisten());};
  },[open,locked]);
  const manage=async(p:VoicePerson,name:string|null,sampleId:string|null)=>{
    if(name===null){audio.clear();await stopAllPlayback();}
    await invoke<void>("manage_voice_person",{personId:p.id,expectedVersion:p.version,name,sampleId});
  };
  const help=<InfoPopover label={standalone?"ボイス一覧":"登録した話者"}><p>声を登録した人物と、登録に使った音声を確認できます。</p><p>声の特徴量はこの端末内に保存し、バックアップにも含めます。削除しても過去の会議に付けた名前は残ります。異なるモデルで登録した声は照合に使いません。モデル変更後は、会議から声を追加登録してください。</p></InfoPopover>;
  const content=<>
    <div className="voice-library-actions"><span>{loaded?`${people.length}人` : busy?"読み込み中…":""}</span><IconButton label="ボイス一覧を再読込" icon="refresh" disabled={locked||busy} onClick={()=>void run(async()=>{})}/></div>
    {loaded&&!people.length&&!error&&<p>登録した話者はいません。</p>}
    {people.map(p=><section className="voice-person" key={p.id} aria-label={p.name}><strong>{p.name}</strong><small>識別子 {p.id.slice(0,8)} · {p.samples.length}区間</small>
      <button className="small-secondary" disabled={locked||busy} onClick={e=>{opener.current=e.currentTarget;setEditing(p);setName(p.name);}}>名前を変更</button>
      <button className="small-secondary" disabled={locked||busy} onClick={e=>{opener.current=e.currentTarget;setDeleting(p);}}>人物と声を削除</button>
      {p.samples.map((s,index)=><div className="voice-sample" key={s.id}>
        <span>登録区間 {index+1} · {s.startUs/1e6}秒〜{s.endUs/1e6}秒 · モデル {s.modelHash.slice(0,8)}<small>{s.sourceAvailable?"特徴量を抽出した元の音声を再生できます。":"登録元の音声がないため再生できません。"}</small></span>
        <button className="small-secondary" disabled={locked||busy||!s.sourceAvailable} aria-label={`${p.name}の登録区間${index+1}を再生`} onClick={()=>void audio.start("",s.id,{voiceSampleId:s.id,expectedStartUs:s.startUs,expectedEndUs:s.endUs})}>登録音声を再生</button>
        <button className="small-secondary" disabled={locked||busy} onClick={()=>void run(()=>manage(p,null,s.id))}>この声を削除</button>
        {audio.selected===s.id&&<div className="voice-sample-player">{audio.controls}</div>}
      </div>)}
    </section>)}
    <ErrorNotice scope="voice" type="VoicePeopleSettings-operation" active={!!(error)}>{error}</ErrorNotice>
  </>;
  return <section className={standalone?"voice-library":"voice-settings"} aria-label={standalone?"ボイス一覧":undefined}>
    {standalone?<><div className="page-heading heading-with-info"><h1>ボイス一覧</h1>{help}</div>{content}</>:<details onToggle={e=>{setOpen(e.currentTarget.open);if(!e.currentTarget.open)audio.clear();}}><summary>登録した話者</summary>{open&&<><div className="heading-with-info"><h3>登録した話者</h3>{help}</div>{content}</>}</details>}
    <CommonModal className="voice-dialog" open={!!editing} blocked={locked||busy} onDismiss={()=>setEditing(null)} restoreTarget={()=>opener.current} aria-label="登録した人物名を変更"><h2>人物名を変更</h2><label>人物名<input maxLength={200} value={name} onChange={e=>setName(e.target.value)} /></label><p>過去の会議に保存した名前は変わりません。</p><button disabled={locked||busy||!name.trim()} onClick={()=>void run(async()=>{await manage(editing!,name,null);setEditing(null);})}>保存</button><ErrorNotice scope="voice" type="VoicePeopleSettings-operation" active={!!(error)}>{error}</ErrorNotice></CommonModal>
    <CommonModal className="voice-dialog" open={!!deleting} blocked={locked||busy} onDismiss={()=>setDeleting(null)} restoreTarget={()=>opener.current} aria-label="人物と声を削除"><h2>「{deleting?.name}」の登録を削除</h2><p>登録した声をすべて削除します。今後の照合には使われません。</p><button disabled={locked||busy} onClick={()=>void run(async()=>{await manage(deleting!,null,null);setDeleting(null);})}>人物と声を削除</button><ErrorNotice scope="voice" type="VoicePeopleSettings-operation" active={!!(error)}>{error}</ErrorNotice></CommonModal>
  </section>;
}
