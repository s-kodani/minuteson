import {listen} from "@tauri-apps/api/event";
import {isTauri} from "@tauri-apps/api/core";
import {useEffect,useState} from "react";
import {IssueNotice,toasts} from "./toast";
import {toastKey} from "./toast-state";

// Recovery of the monitor must never clear a meeting/startup operation error,
// a tray creation failure, or another residency operation's diagnostic.
export function ResidencyNotifications({scope}:{scope:string}) {
  const [error,setError]=useState(""),[monitorError,setMonitorError]=useState<string|null>(null);
  useEffect(()=>{
    if(!isTauri())return;
    let disposed=false;const stops:(()=>void)[]=[];
    const subscribe=<T,>(name:string,accept:(payload:T)=>void)=>{
      void listen<T>(name,event=>{if(!disposed)accept(event.payload);}).then(stop=>{if(disposed)stop();else stops.push(stop);}).catch(()=>{});
    };
    subscribe<string>("residency-error",setError);
    subscribe<string|null>("residency-monitor-status",setMonitorError);
    return()=>{disposed=true;stops.forEach(stop=>stop());};
  },[]);
  useEffect(()=>()=>{toasts.resolve(toastKey(scope,"residency-monitor"));toasts.resolve(toastKey(scope,"residency-operation"));},[scope]);
  return <><IssueNotice scope={scope} type="residency-operation" label="常駐の操作" message={error}/><IssueNotice scope={scope} type="residency-monitor" label="常駐監視" message={monitorError}/></>;
}
