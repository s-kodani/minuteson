import { useEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";
import { invoke } from "./ipc";
import "./credits.css";

interface Credit { name:string; version:string; license:string; sources:string[] }
interface Notice { title:string; text:string }
interface CreditsData { technologies:Credit[]; models:Credit[]; dependencies:Credit[]; notices:Notice[] }

function CreditList({ entries }: { entries:Credit[] }) {
  return <ul className="credits-list">{entries.map((entry,index)=><li key={`${entry.name}:${entry.version}:${index}`}>
    <div className="credits-entry-heading"><strong>{entry.name}</strong><span>{entry.version}</span></div>
    <p className="credits-license">{entry.license}</p>
    {entry.sources.length>0&&<details className="credits-sources"><summary>出典</summary><ul>{entry.sources.map(source=><li key={source}>{source}</li>)}</ul></details>}
  </li>)}</ul>;
}
function NoticeText({ notice }: { notice:Notice }) {
  const [expanded,setExpanded]=useState(false);
  return <details className="credits-notice" onToggle={event=>setExpanded(event.currentTarget.open)}>
    <summary>{notice.title}</summary>{expanded&&<pre tabIndex={0}>{notice.text}</pre>}
  </details>;
}

/** Static build attribution: opening reads existing resources and never starts
 * model preparation, downloading, or a meeting operation. */
export function Credits() {
  const [open,setOpen]=useState(false), [data,setData]=useState<CreditsData|null>(null);
  const [loading,setLoading]=useState(false), [error,setError]=useState("");
  const alive=useRef(true), requesting=useRef(false);
  const trigger=useRef<HTMLButtonElement>(null);
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
  const load=async()=>{
    if(requesting.current)return;
    requesting.current=true;setLoading(true);setError("");
    try {
      const result=await invoke<CreditsData>("get_credits");
      if(!result||![result.technologies,result.models,result.dependencies,result.notices].every(Array.isArray))throw Error("クレジットの形式が不正です。");
      if(alive.current)setData(result);
    } catch(cause) { if(alive.current)setError(String(cause)); }
    finally {requesting.current=false;if(alive.current)setLoading(false);}
  };
  const show=()=>{setOpen(true);if(!data)void load();};
  return <>
    <button ref={trigger} type="button" className="settings-footer-link" aria-haspopup="dialog" aria-expanded={open} onClick={show}>クレジット</button>
    <CommonModal portal open={open} onDismiss={()=>setOpen(false)} restoreTarget={()=>trigger.current} dismissLabel="閉じる" dismissText="クレジットを閉じる" aria-label="クレジット" className="credits-modal">
      <h2>クレジット</h2>
      {loading&&<p role="status">クレジットを読み込んでいます。</p>}
      {error&&<div className="credits-error"><p role="alert">クレジットを読み込めませんでした。{error}</p><button type="button" className="small-secondary" disabled={loading} onClick={()=>void load()}>再試行</button></div>}
      {data&&<div className="credits-content">
        <section aria-label="主要な利用技術"><h3>主要な利用技術</h3><CreditList entries={data.technologies}/></section>
        <section aria-label="AIモデル"><h3>AIモデル</h3><p className="credits-description">公式カタログに掲載しているモデルです。</p><CreditList entries={data.models}/></section>
        <section aria-label="依存ライブラリ"><h3>依存ライブラリ</h3><details><summary>ライブラリ一覧（{data.dependencies.length}件）</summary><CreditList entries={data.dependencies}/></details></section>
        <section aria-label="ライセンス・通知文"><h3>ライセンス・通知文</h3><p className="credits-description">このアプリに同梱されているライセンスと通知文です。</p>{data.notices.map((notice,index)=><NoticeText key={`${notice.title}:${index}`} notice={notice}/>)}</section>
      </div>}
    </CommonModal>
  </>;
}
