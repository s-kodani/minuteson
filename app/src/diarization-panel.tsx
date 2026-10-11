import { IconButton } from "./action-icon";
import { invoke,isTauri } from "@tauri-apps/api/core";
import { InfoPopover } from "./info-popover";
import { OptionsModal } from "./options-modal";
import type { ModalReplacement } from "./common-modal";
import { IssueNotice } from "./toast";
import { useActionConfirmation } from "./confirmation-service";
import { useEffect,useId,useRef,useState,type ReactNode } from "react";
import { DraftSaveQueue,registerPendingEdits,type VersionedDraft } from "./draft-save";
import { canRetryJob,jobLabel,type AnalysisJob,type VersionState } from "./analysis-state";
import { JobModelRebinding } from "./model-rebinding";
import { diarizationConfigured,fixedDiarizationInput,fixedTracks,trackLabel,type DiarizationEngine,type DiarizationPreferences,type DiarizationTrack } from "./diarization-state";
type Preferences = VersionedDraft<DiarizationPreferences>;
export function DiarizationPanel({meetingId,meetingTitle,recorded,recordingActive,locked,jobs,versions,withSavedEdits,refresh,visible=true,renderSettings}: {
  meetingId:string;meetingTitle:string;recorded:boolean;recordingActive:boolean;locked:boolean;jobs:AnalysisJob[];versions:VersionState|null;
  withSavedEdits:(action:()=>void|Promise<void>)=>Promise<void>;refresh:()=>Promise<void>;
  visible?:boolean;
  renderSettings?:(entry:ReactNode,view:ModalReplacement|null,persistentBody:ReactNode)=>ReactNode;
}) {
  const profileSelectId = useId();
  const engineTitleId = useId();
  const engineTrigger = useRef<HTMLButtonElement>(null);
  const [engineOpen,setEngineOpen] = useState(false);
  const countTitleId = useId();
  const countTrigger = useRef<HTMLButtonElement>(null);
  const [countsOpen,setCountsOpen] = useState(false);
  useEffect(()=>{if(!visible){setCountsOpen(false);setEngineOpen(false);}},[visible]);
  const confirm = useActionConfirmation(locked);
  const [queue] = useState(()=>new DraftSaveQueue<DiarizationPreferences,Preferences>(
    (_id,expectedLockVersion,payload)=>invoke("save_diarization_preferences",{expectedLockVersion,payload}),payload=>structuredClone(payload)));
  const [settings,setSettings]=useState(()=>queue.snapshot());
  const [profile,setProfile]=useState<"standard"|"highAccuracy">("standard");
  const [tracks,setTracks]=useState<DiarizationTrack[]>([]);
  const [counts,setCounts]=useState<Record<string,string>>({});
  const [busy,setBusy]=useState(false),[error,setError]=useState("");
  const [trackError,setTrackError]=useState("");
  const [trackRead,setTrackRead]=useState(0);
  const mounted=useRef(true),retryOperations=useRef(new Map<string,string>()),startOperations=useRef(new Map<string,string>());
  const busyRef=useRef(false);
  useEffect(()=>{
    mounted.current=true;
    const unsub=queue.subscribe(()=>setSettings(queue.snapshot())),unregister=registerPendingEdits(()=>queue.flush());
    if(isTauri()) void invoke<Preferences>("get_diarization_preferences").then(p=>{if(mounted.current)queue.load(p);}).catch(e=>{if(mounted.current)setError(String(e));});
    return()=>{mounted.current=false;unsub();unregister();};
  },[queue]);
  useEffect(()=>{
    if(!settings.dirty||settings.saving||settings.error||confirm.pending)return;
    const timer=window.setTimeout(()=>{void queue.flush().catch(()=>{});},700);return()=>window.clearTimeout(timer);
  },[queue,settings,confirm.pending]);
  useEffect(()=>{
    let disposed=false;setTracks([]);
    if(!recorded)setTrackError("");
    if(recorded&&isTauri()) void invoke<{tracks:DiarizationTrack[]}>("get_diarization_tracks",{meetingId}).then(p=>{if(!disposed){setTracks(p.tracks);setTrackError("");}}).catch(e=>{if(!disposed)setTrackError(String(e));});
    return()=>{disposed=true;};
  },[meetingId,recorded,trackRead]);
  const disabled=busy||locked;
  const pending=jobs.some(j=>["queued","running","pausing"].includes(j.state));
  const engine=settings.payload?.[profile];
  const update=(patch:Partial<DiarizationEngine>)=>queue.edit(p=>({...p,[profile]:{...p[profile],...patch}}));
  const run=async(action:()=>Promise<void>)=>{
    if(busyRef.current||locked)return;busyRef.current=true;setBusy(true);setError("");
    try {await action();} catch(e) {if(mounted.current)setError(String(e));} finally {busyRef.current=false;if(mounted.current)setBusy(false);}
  };
  const reloadSettings=()=>{
    if(disabled||queue.snapshot().saving)return;
    const expected = JSON.stringify(queue.snapshot().payload);
    const read = async () => { queue.load(await invoke<Preferences>("get_diarization_preferences")); };
    void run(async()=>{ if (!queue.snapshot().dirty) await read(); else await confirm({ title: "話者分離設定の入力破棄を確認", target: "端末内の話者分離設定", confirmLabel: "入力を破棄して再読込する", message: "未保存の話者分離設定を破棄して、保存された設定を再読込します。この入力は復元できません。ジョブの固定条件は変更しません。", validate: () => JSON.stringify(queue.snapshot().payload) === expected, onError: cause => { if (mounted.current) setError(String(cause)); }, action: read }); });
  };
  const saved=async(action:()=>Promise<void>)=>{
    let ran=false,failed=false,failure:unknown;
    await withSavedEdits(async()=>{ran=true;try{await action();}catch(cause){failed=true;failure=cause;}});
    if(!ran)throw Error("編集の保存を完了できませんでした。入力を保持し、操作は開始していません。");
    if(failed)throw failure;
  };
  const start=()=>run(()=>saved(async()=>{
    const engine=queue.snapshot().payload?.[profile];
    if(!engine||!diarizationConfigured(engine))throw Error("話者分離エンジンと両モデルの絶対パス・設定を確認してください。");
    const [state,latestJobs]=await Promise.all([invoke<VersionState>("get_analysis_versions",{meetingId}),invoke<AnalysisJob[]>("get_analysis_jobs",{meetingId})]);
    if(latestJobs.some(j=>["queued","running","pausing"].includes(j.state)))throw Error("先行する処理の完了または一時停止を待ってください。");
    const request={input:fixedDiarizationInput(state,meetingId,"pending"),...engine,profile:profile==="highAccuracy"?"high_accuracy":"standard",tracks:fixedTracks(tracks,counts)};
    const key=JSON.stringify(request),operation=startOperations.current.get(key)??crypto.randomUUID();startOperations.current.set(key,operation);
    request.input.operationId=operation;
    try {await invoke("start_diarization",{request});startOperations.current.delete(key);} finally {await refresh();}
  }));
  const retry=(job:AnalysisJob)=>{
    void run(async()=>{
      await confirm({ title: "話者分離の再試行を確認", target: `会議「${meetingTitle}」 · ジョブ ${job.id}`, confirmLabel: "同じ条件で再試行する", message: `同じ固定入力・モデル・人数で話者分離を再試行します。保存済み ${job.completedUnits} 処理単位を再照合して引き継ぎます。連続音声はトラック、欠損付き音声は保存区間ごとの単位です。今の設定画面は使用せず、現在版と編集を上書きしません。`, onError: cause => { if (mounted.current) setError(String(cause)); }, action: async () => {
      const operationId=retryOperations.current.get(job.id)??crypto.randomUUID();retryOperations.current.set(job.id,operationId);
      try {await invoke("retry_diarization",{request:{meetingId,jobId:job.id,operationId,expectedSpecHash:job.specHash}});retryOperations.current.delete(job.id);} finally {await refresh();}
      } });
    });
  };
  const cancelJob = (job: AnalysisJob) => run(async () => { await confirm({ title: "話者分離の取消を確認", target: `会議「${meetingTitle}」 · ジョブ ${job.id}`, confirmLabel: "話者分離を取り消す", message: "話者分離を取り消します。録音・現在版・編集は削除しません。", onError: cause => { if (mounted.current) setError(String(cause)); }, action: async () => { await invoke("cancel_analysis_job", { jobId: job.id }); await refresh(); } }); });
  const jobAction=(command:string,jobId:string)=>run(async()=>{await invoke(command,{jobId});await refresh();});
  const current=versions?.versions.find(v=>v.id===versions.currentId);
  const engineContent = <div className="analysis-engine-view" hidden={!engineOpen}>
    <h2 id={engineTitleId}>sherpa-onnx 設定（端末内に保存）</h2><div className="options-dialog-content">
      <p>導入済みCLI・区間モデル・埋め込みモデルを指定します。CPUで処理します。品質・10話者・16GB端末・4時間の推論は未検証で、プロファイル名だけで精度を保証しません。</p>
      <div className="info-field"><div className="heading-with-info"><label htmlFor={profileSelectId}>話者分離プロファイル</label><InfoPopover label="話者分離の標準／高精度">モデルと数値設定を別々に保存するプロファイルです。CPUで処理し、負荷はモデルと設定に依存します。品質・10話者・16GB端末・4時間の推論は未検証です。</InfoPopover></div><select id={profileSelectId} value={profile} disabled={disabled} onChange={e=>setProfile(e.target.value as typeof profile)}><option value="standard">標準</option><option value="highAccuracy">高精度</option></select></div>
      {engine?<fieldset disabled={disabled}>
        <label>話者分離CLIの絶対パス<input value={engine.executablePath} maxLength={4096} onChange={e=>update({executablePath:e.target.value})}/></label>
        <label>区間モデルの絶対パス<input value={engine.segmentationModelPath} maxLength={4096} onChange={e=>update({segmentationModelPath:e.target.value})}/></label>
        <label>埋め込みモデルの絶対パス<input value={engine.embeddingModelPath} maxLength={4096} onChange={e=>update({embeddingModelPath:e.target.value})}/></label>
        <label>同梱ランタイムの絶対パス（1行1ファイル）<textarea value={engine.runtimeLibraryPaths.join("\n")} maxLength={65536} rows={3} onChange={e=>update({runtimeLibraryPaths:e.target.value===""?[]:e.target.value.split("\n")})}/></label>
        <p>共有版では同梱ライブラリをすべて指定してください。静的版は空にできます。依存ライブラリの自動発見は未実装です。</p>
        {([ ["threads","話者分離CPUスレッド数",1,64], ["clusterThresholdMilli","クラスタ閾値（1000分の1単位）",0,2000], ["windowShiftMilli","窓移動比率（1000分の1単位）",1,1000], ["minDurationOnMs","最小発話長（ms）",0,60000], ["minDurationOffMs","結合ギャップ（ms）",0,60000] ] as const).map(([key,label,min,max])=><label key={key}>{label}<input type="number" min={min} max={max} step={1} value={engine[key]} onChange={e=>update({[key]:Number(e.target.value)})}/></label>)}
      </fieldset>:<p>{error?"話者分離設定を読込できませんでした。":"話者分離設定を読込中です。"}</p>}
      <div className="analysis-actions"><span role="status">{!engine?"話者分離設定は未読込":settings.error?"話者分離設定の保存に失敗":settings.saving?"話者分離設定を保存中":settings.dirty?"話者分離設定は未保存":"話者分離設定は保存済み"}</span><button type="button" className="small-secondary" disabled={disabled||settings.saving||!settings.dirty} onClick={()=>void run(()=>queue.retry())}>話者分離設定を保存・再試行</button><IconButton label="話者分離設定を再読込" icon="refresh" type="button" className="small-secondary" disabled={disabled||settings.saving||(!engine&&!error)} onClick={reloadSettings} /></div>
      <IssueNotice scope={meetingId} type="diarization-settings" label="話者分離設定の保存" message={settings.error} toastMessage="話者分離設定を保存できません。入力は保持しています" action={{kind:"retry",label:"話者分離設定保存を再試行",disabled:disabled||settings.saving,run:()=>run(()=>queue.retry())}} />
    </div></div>;
  const panelContent = <section className="diarization-panel" aria-label="話者分離操作">
    <div className="heading-with-info"><h3>話者分離と人数指定</h3><InfoPopover label="話者分離">採用済み文字起こしと編集を固定し、新しい候補を作ります。本文・名前・手動割り当ては保持します。欠損付き音声は保存区間を別々に解析し、同じトラック内の特徴量で照合します。特徴量不足は区間ごとの未照合話者として残します。別の区間やトラックの同じ番号は同一人物を意味しません。品質は未検証です。</InfoPopover></div>
    <div className="options-modal analysis-settings"><button ref={engineTrigger} type="button" className="small-secondary options-modal-trigger" aria-label="sherpa-onnx 設定（端末内に保存）" aria-haspopup="dialog" aria-expanded={engineOpen} onClick={event=>{event.currentTarget.focus({preventScroll:true});setEngineOpen(true);}}>sherpa-onnx 設定（端末内に保存）</button></div>
    <div className="heading-with-info"><h3>話者の人数</h3><InfoPopover label="話者の人数">人数はトラック全体に指定し、通常は自動推定です。欠損付き音声でも各保存区間に同じ人数を割り当てません。会議全体の参加者総数とは異なります。今回の開始条件に固定され、受付上限100人は精度の検証済み範囲ではありません。</InfoPopover></div>
    <fieldset disabled={disabled||pending||recordingActive} className="diarization-tracks">
      {tracks.map(t=><div key={t.id}><label>{trackLabel(t.kind)} 人数<select value={(counts[t.id]??"auto")==="auto"?"auto":"fixed"} onChange={e=>setCounts(c=>({...c,[t.id]:e.target.value==="auto"?"auto":"3"}))}><option value="auto">自動推定</option><option value="fixed">人数を指定</option></select></label>{(counts[t.id]??"auto")!=="auto"&&<label>{trackLabel(t.kind)}の指定人数<input type="number" min={1} max={100} step={1} value={counts[t.id]} onChange={e=>setCounts(c=>({...c,[t.id]:e.target.value}))}/></label>}</div>)}
    </fieldset>
    <IssueNotice scope={meetingId} type="diarization-tracks" label="話者分離トラックの読込" message={trackError} toastMessage="話者分離トラックを読み込めません" action={{kind:"retry",label:"トラックを再読込",disabled,run:()=>setTrackRead(n=>n+1)}}><IconButton label="トラックを再読込" icon="refresh" type="button" disabled={disabled} onClick={()=>setTrackRead(n=>n+1)} /></IssueNotice>
    {!current&&<p>文字起こしの候補を確認・採用してから開始してください。</p>}
    {current&&Array.isArray(current.config.tracks)&&<p>現在版に保存された人数条件: {JSON.stringify(current.config.tracks)}</p>}
    <button type="button" disabled={disabled||!recorded||recordingActive||pending||!current||!engine||!diarizationConfigured(engine)||!tracks.length} onClick={()=>void start()}>話者分離を開始・やり直す</button>
    <ul className="analysis-jobs" aria-label="話者分離ジョブ">{jobs.filter(j=>j.kind==="diarization").map(j=><li key={j.id}><div><strong>{jobLabel(j.state)}{j.attempt>1?` · 試行 ${j.attempt}`:""}</strong><span>保存済み {j.completedUnits} / {j.totalUnits??"?"} 処理単位{j.pauseReason==="recording"?" · 録音・取り込み優先で停止":""}</span><small>ジョブ {j.id.slice(0,8)}{j.retryOfJobId?` · 再試行元 ${j.retryOfJobId.slice(0,8)}`:""}</small>{j.errorCode&&<span className="action-error">{j.errorCode}</span>}</div><div className="analysis-actions">
      {["queued","running"].includes(j.state)&&<IconButton label="話者分離を一時停止" icon="pause" type="button" disabled={disabled} onClick={()=>void jobAction("pause_analysis_job",j.id)} />}
      {["paused","interrupted"].includes(j.state)&&<button type="button" disabled={disabled||recordingActive||pending} onClick={()=>void jobAction("resume_analysis_job",j.id)}>話者分離を再開</button>}
      {["queued","running","paused","interrupted"].includes(j.state)&&<button type="button" disabled={disabled} onClick={()=>void cancelJob(j)}>話者分離を取消</button>}
      {canRetryJob(j,jobs,"diarization")&&<button type="button" disabled={disabled||recordingActive||pending} onClick={()=>retry(j)}>話者分離を同じ条件で再試行</button>}
      <JobModelRebinding meetingId={meetingId} job={j} jobs={jobs} locked={disabled} recordingActive={recordingActive} withSavedEdits={withSavedEdits} refresh={refresh} />
    </div></li>)}</ul>
    <IssueNotice scope={meetingId} type="diarization-operation" label="話者分離操作" message={error} toastMessage="話者分離操作を完了できません。状態を確認してください" />
  </section>;
  const engineView:ModalReplacement|null = engineOpen ? {
    key:"diarization-engine",parentKey:renderSettings?"diarization-counts":undefined,labelledBy:engineTitleId,content:null,className:"diarization-engine-dialog",blocked:disabled,
    onDismiss:()=>setEngineOpen(false),restoreTarget:()=>engineTrigger.current,dismissLabel:"閉じる",
  } : null;
  if(renderSettings){
    const entry=<div className="options-modal speaker-diarization-tools" hidden={!visible}><button ref={countTrigger} type="button" className="small-secondary options-modal-trigger" aria-label="人数指定・話者分離" aria-haspopup="dialog" aria-expanded={countsOpen&&visible} onClick={event=>{event.currentTarget.focus({preventScroll:true});setCountsOpen(true);}}>人数指定・話者分離</button></div>;
    const body=<><div className="diarization-count-view" hidden={!countsOpen||engineOpen}><h2 id={countTitleId}>人数指定・話者分離</h2><div className="options-dialog-content">{panelContent}</div></div>{engineContent}</>;
    const view:ModalReplacement|null=countsOpen&&visible ? engineView ?? {
      key:"diarization-counts",labelledBy:countTitleId,content:null,className:"diarization-count-dialog",blocked:disabled,
      onDismiss:()=>setCountsOpen(false),restoreTarget:()=>countTrigger.current,dismissLabel:"閉じる",
    } : null;
    return renderSettings(entry,view,body);
  }
  return <OptionsModal className="speaker-diarization-tools" label="人数指定・話者分離" hidden={!visible} persistentReplacement={engineContent} replacement={engineView}>{panelContent}</OptionsModal>;
}
