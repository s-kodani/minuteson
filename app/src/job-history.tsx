import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke, isTauri } from "./ipc";
import { CommonModal } from "./common-modal";
import { IconButton } from "./action-icon";
import { operationName } from "./operation-progress-state";
import { displayNativePathsInText } from "./native-path";
import { jobFailureDetail } from "./job-notification-state";
import { checkedHistoryPage, historyElapsed, historyKinds, historyStates, mergeHistory, type HistoryCursor, type HistoryState, type JobHistoryEntry } from "./job-history-state";
import "./job-history.css";

const date=(time:number|null)=>time===null?"未確定":new Date(time).toLocaleString("ja-JP");
export function JobHistory({locked,onOpenMeeting,onOpenModels,onOpenData,onShowRestored}:{locked:boolean;onOpenMeeting:(id:string)=>Promise<void>;onOpenModels:()=>void;onOpenData?:()=>void;onShowRestored?:(ids:string[])=>Promise<void>}){
  const [kind,setKind]=useState(""),[state,setState]=useState("");
  const [rows,setRows]=useState<JobHistoryEntry[]>([]),[cursor,setCursor]=useState<HistoryCursor|null>(null);
  const [busy,setBusy]=useState(false),[error,setError]=useState(""),[storageError,setStorageError]=useState("");
  const [loaded,setLoaded]=useState(false),[changed,setChanged]=useState(false),[selected,setSelected]=useState<JobHistoryEntry|null>(null);
  const generation=useRef(0),active=useRef(true),reading=useRef(false),opener=useRef<HTMLElement|null>(null);
  const olderPage=useRef(false),needsRead=useRef(false),selectedEntry=useRef(selected);selectedEntry.current=selected;
  const latest=useRef({kind,state});latest.current={kind,state};
  const read=async(older=false)=>{
    if(reading.current)return;
    const request=++generation.current,filter={...latest.current};reading.current=true;setBusy(true);setError("");
    try{
      const next=checkedHistoryPage(await invoke("list_job_history",{query:{...(filter.kind?{kind:filter.kind}:{}),...(filter.state?{state:filter.state}:{}),...(older&&cursor?{cursor}:{}),limit:50}}));
      if(active.current&&request===generation.current&&filter.kind===latest.current.kind&&filter.state===latest.current.state){
        olderPage.current=older;
        setRows(prior=>older?mergeHistory(prior,next.items):next.items);setCursor(next.nextCursor);setStorageError(next.storageError??"");setLoaded(true);setChanged(false);
        if(!older)setSelected(prior=>prior&&!next.items.some(e=>e.id===prior.id)?null:prior);
      }
    }catch(cause){if(active.current&&request===generation.current)setError(String(cause));}
    finally{reading.current=false;if(active.current){setBusy(false);if(needsRead.current&&!olderPage.current&&!selectedEntry.current){needsRead.current=false;window.setTimeout(()=>{if(active.current)void refresh.current();},0);}}}
  };
  const refresh=useRef(read);refresh.current=read;
  useEffect(()=>{
    active.current=true;
    let stopped=false;let stop:(()=>void)|undefined;
    const changed=()=>{
      if(stopped)return;
      if(olderPage.current||selectedEntry.current){setChanged(true);return;}
      if(reading.current){needsRead.current=true;return;}
      void refresh.current();
    };
    if(isTauri())void listen("job-history-changed",changed).then(fn=>{if(stopped)fn();else stop=fn;}).catch(()=>{});
    const timer=window.setInterval(changed,24*60*60*1000);window.addEventListener("focus",changed);
    return()=>{active.current=false;stopped=true;stop?.();clearInterval(timer);window.removeEventListener("focus",changed);generation.current++;};
  },[]);
  useEffect(()=>{
    let disposed=false;
    const load=()=>{if(disposed)return;if(reading.current){timer=window.setTimeout(load,50);return;}void refresh.current();};
    let timer:number|undefined;load();return()=>{disposed=true;if(timer!==undefined)clearTimeout(timer);};
  },[kind,state]);
  return <section className="job-history" aria-label="過去のジョブ履歴">
    <div className="job-history-filters"><label>種類<select aria-label="履歴の種類" value={kind} onChange={e=>{setSelected(null);setKind(e.target.value);}}><option value="">すべての種類</option>{historyKinds.map(k=><option value={k} key={k}>{operationName(k)}</option>)}</select></label><label>結果<select aria-label="履歴の結果" value={state} onChange={e=>{setSelected(null);setState(e.target.value);}}><option value="">すべての結果</option>{Object.entries(historyStates).map(([key,label])=><option key={key} value={key}>{label}</option>)}</select></label><div className="job-history-actions"><span>直近90日間を保持</span><IconButton label="ジョブ履歴を再読込" icon="refresh" disabled={busy} onClick={()=>void read()}/></div></div>
    {changed&&<p role="status">履歴が更新されています。<button type="button" className="small-secondary" disabled={busy} onClick={()=>void read()}>最新の履歴を表示</button></p>}
    {error&&<p role="alert">{error} <button type="button" className="small-secondary" disabled={busy} onClick={()=>void read()}>再試行</button></p>}
    {storageError&&<p role="alert">履歴の保存に問題があります。{displayNativePathsInText(storageError)}</p>}
    {busy&&!loaded&&<p role="status">ジョブ履歴を読み込んでいます。</p>}
    {loaded&&!rows.length&&!error&&<div className="job-list-empty"><h2>該当する履歴はありません</h2><p>過去90日間の処理結果がここに表示されます。</p></div>}
    {rows.length>0&&<><div className="job-history-summary"><span>{rows.length} 件を表示</span><span>新しい順</span></div><ul className="job-history-rows">{rows.map(entry=><li key={entry.id}><div className="job-history-target"><small>{operationName(entry.kind)}</small><strong>{entry.target||operationName(entry.kind)}</strong></div><span className={`job-history-state job-history-${entry.state}`}>{historyStates[entry.state]}</span><div className="job-history-time"><span>受付 {date(entry.acceptedAt)}</span><span>終了 {date(entry.finishedAt)}</span></div><span className="job-history-elapsed">{historyElapsed(entry)}</span><button type="button" className="small-secondary" aria-label={`${operationName(entry.kind)}・${entry.target}の詳細`} onClick={e=>{opener.current=e.currentTarget;setSelected(entry);}}>詳細</button></li>)}</ul></>}
    {cursor&&<div className="job-history-more"><button type="button" className="small-secondary" disabled={busy} onClick={()=>void read(true)}>{busy?"読み込み中…":"さらに読み込む"}</button></div>}
    <CommonModal open={!!selected} onDismiss={()=>setSelected(null)} restoreTarget={()=>opener.current} className="job-history-dialog" aria-labelledby="job-history-detail-title" dismissLabel="閉じる">
      <h2 id="job-history-detail-title">ジョブの詳細</h2>
      {selected&&<><p><strong>{selected.target||operationName(selected.kind)}</strong></p><dl><dt>種類</dt><dd>{operationName(selected.kind)}</dd><dt>結果</dt><dd>{historyStates[selected.state as HistoryState]}</dd><dt>受付日時</dt><dd>{date(selected.acceptedAt)}</dd><dt>終了日時</dt><dd>{date(selected.finishedAt)}</dd><dt>経過時間</dt><dd>{historyElapsed(selected)}</dd></dl><p>{displayNativePathsInText(selected.detail?jobFailureDetail(selected.detail):(selected.state==="succeeded"?"処理が完了しました。":selected.state==="cancelled"?"処理を取り消しました。":"現在の処理結果を確認してください。"))}</p><div className="analysis-actions">
        {selected.meetingId&&<button type="button" className="small-secondary" disabled={locked} onClick={()=>{const id=selected.meetingId!;setSelected(null);void onOpenMeeting(id);}}>会議を開く</button>}
        {selected.kind==="model"&&<button type="button" className="small-secondary" disabled={locked} onClick={()=>{setSelected(null);onOpenModels();}}>モデル設定を開く</button>}
        {["backup","restore","deletion","audio_deletion","model_deletion"].includes(selected.kind)&&onOpenData&&<button type="button" className="small-secondary" disabled={locked} onClick={()=>{setSelected(null);onOpenData();}}>設定で結果を確認</button>}
        {!!selected.relatedMeetingIds?.length&&onShowRestored&&<button type="button" className="small-secondary" disabled={locked} onClick={()=>{const ids=selected.relatedMeetingIds!;setSelected(null);void onShowRestored(ids);}}>復元した会議を開く</button>}
      </div></>}
    </CommonModal>
  </section>;
}
