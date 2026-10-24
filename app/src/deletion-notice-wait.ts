import {isTauri} from "@tauri-apps/api/core";
import {listen} from "@tauri-apps/api/event";
import {useEffect,useState} from "react";

export function useDeletionNoticeWait(){
  const [waiting,setWaiting]=useState(false);
  useEffect(()=>{
    if(!isTauri())return;
    let disposed=false,stop:(()=>void)|undefined;
    const owners=new Set<string>();
    void listen<{id:string;waiting:boolean}>("deletion-notice-wait",event=>{
      const value=event.payload;
      if(disposed||!value||typeof value.id!=="string"||typeof value.waiting!=="boolean")return;
      if(value.waiting)owners.add(value.id);else owners.delete(value.id);
      setWaiting(owners.size>0);
    })
      .then(unlisten=>{if(disposed)unlisten();else stop=unlisten;}).catch(()=>{});
    return()=>{disposed=true;stop?.();};
  },[]);
  return waiting;
}
