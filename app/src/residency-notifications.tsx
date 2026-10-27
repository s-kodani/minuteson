import {listen} from "@tauri-apps/api/event";
import {isTauri} from "@tauri-apps/api/core";
import {useEffect,useState} from "react";
import {IssueNotice,toasts} from "./toast";
import {toastKey} from "./toast-state";

// The native monitor logs and retries independently. Only failed operations
// requiring the user's attention belong in the notification menu.
export function ResidencyNotifications({scope}:{scope:string}) {
  const [error,setError]=useState("");
  useEffect(()=>{
    if(!isTauri())return;
    let disposed=false;const stops:(()=>void)[]=[];
    const subscribe=<T,>(name:string,accept:(payload:T)=>void)=>{
      void listen<T>(name,event=>{if(!disposed)accept(event.payload);}).then(stop=>{if(disposed)stop();else stops.push(stop);}).catch(()=>{});
    };
    subscribe<string>("residency-error",setError);
    return()=>{disposed=true;stops.forEach(stop=>stop());};
  },[]);
  useEffect(()=>()=>toasts.resolve(toastKey(scope,"residency-operation")),[scope]);
  return <IssueNotice scope={scope} type="residency-operation" label="常駐の操作" message={error}/>;
}
