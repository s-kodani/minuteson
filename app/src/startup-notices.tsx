import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke, isTauri } from "./ipc.ts";
import { CommonModal } from "./common-modal";
import { topOpenModal } from "./modal-state";
import { ToastNotice } from "./toast";
import { checkedStartupNotices, type StartupNotice } from "./startup-notice-state";

/** Presentation only. Feature controllers retain execution and confirmation. */
export function StartupNoticeHost({ ready, locked, refreshKey, onReview, onDismissReview, renderAction }: {
  ready: boolean; locked: boolean; refreshKey: string;
  onReview: (notice: StartupNotice) => void;
  onDismissReview: () => void;
  renderAction: (notice: StartupNotice, dismiss: () => void) => ReactNode;
}) {
  const [notices,setNotices] = useState<StartupNotice[]>([]), [open,setOpen] = useState(false), [selected,setSelected] = useState<StartupNotice|null>(null);
  const [error,setError] = useState(""), [busy,setBusy] = useState(false);
  const [modalEpoch,setModalEpoch] = useState(0);
  const auto = useRef(false), live = useRef(true), generation = useRef(0);
  const latest = useRef({locked,onReview,onDismissReview}); latest.current = {locked,onReview,onDismissReview};
  const load = async () => {
    const epoch = ++generation.current;
    try { const rows = checkedStartupNotices(await invoke("get_startup_notices")); if(live.current && epoch===generation.current){setNotices(rows);setError("");} }
    catch(cause){if(live.current && epoch===generation.current)setError(String(cause));}
  };
  useEffect(()=>{live.current=true;if(isTauri() && ready && topOpenModal()?.getAttribute("aria-labelledby")!=="startup-recovery-title")void load();return()=>{live.current=false;generation.current++;};},[refreshKey,ready,modalEpoch]);
  useEffect(()=>{const closed=()=>setModalEpoch(v=>v+1);document.addEventListener("close",closed,true);return()=>document.removeEventListener("close",closed,true);},[]);
  const show = (rows: StartupNotice[], choice: StartupNotice|null) => {
    if(latest.current.locked || topOpenModal())return;
    setSelected(choice);setOpen(true);
    if(choice && choice.kind!=="information")latest.current.onReview(choice);
    // Admission is presentation-only and lasts until native process exit.
    void Promise.all(rows.map(n=>invoke("respond_startup_notice",{id:n.id,revision:n.revision,response:"later"}))).catch(cause=>{if(live.current)setError(String(cause));});
  };
  useEffect(()=>{
    if(!ready || !notices.length || auto.current)return;
    if(topOpenModal()?.getAttribute("aria-labelledby")==="startup-recovery-title")return;
    auto.current=true;
    const rows=notices.filter(n=>!n.deferred);
    if(rows.length && !locked && !topOpenModal() && document.visibilityState==="visible")show(rows,rows.length===1?rows[0]:null);
    // Busy startup is not a reason to interrupt the user later in this session.
  },[notices,ready,locked,modalEpoch]);
  const dismiss=()=>{setOpen(false);setSelected(null);latest.current.onDismissReview();};
  useEffect(()=>{if(locked&&open)dismiss();},[locked,open]);
  const acknowledge=async(n:StartupNotice)=>{
    if(locked||busy)return;
    setBusy(true);
    try{await invoke("respond_startup_notice",{id:n.id,revision:n.revision,response:"acknowledge"});dismiss();await load();}
    catch(cause){setError(String(cause));}finally{setBusy(false);}
  };
  return <>
    {notices.map(n=><ToastNotice key={`${n.id}:${n.revision}`} scope="startup-notices" type={n.id} eventId={n.revision} level="info" historyOnly message={n.title} details={n.message}
      action={{kind:"open",label:"案内を開く",disabled:locked||busy,run:()=>show([n],n)}} clearHistoryOnResolve />)}
    <ToastNotice scope="startup-notices" type="read" level="error" message={error} action={{kind:"retry",label:"案内を再確認",disabled:locked||busy,run:load}} />
    <CommonModal portal open={open} blocked={busy} onDismiss={dismiss} dismissLabel="閉じる" dismissText="後で" restoreTarget={()=>document.querySelector<HTMLElement>('.header-notification-button')} aria-labelledby="startup-notices-title" className="startup-notices-dialog model-license-dialog">
      <h2 id="startup-notices-title">{selected?.title??"起動時の案内"}</h2>
      {selected?<><p>{selected.message}</p>{selected.kind==="information"?<div className="analysis-actions"><button disabled={busy||locked} onClick={()=>void acknowledge(selected)}>確認しました</button><button onClick={dismiss}>後で</button></div>:renderAction(selected,dismiss)}</>
        :<ul>{notices.map(n=><li key={n.id}><button disabled={locked||busy} onClick={()=>{setSelected(n);if(n.kind!=="information")latest.current.onReview(n);}}>{n.title}</button></li>)}</ul>}
    </CommonModal>
  </>;
}
