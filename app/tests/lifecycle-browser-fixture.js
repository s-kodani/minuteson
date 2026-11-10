// Production UI with synthetic lifecycle persistence; no native DB/model/audio.
import { emit } from "@tauri-apps/api/event";
const key="MON_LIFECYCLE_GUIDANCE";
const f=window.lifecycleFixture={calls:[],pending:localStorage.getItem(key)==="pending"||(localStorage.getItem(key)===null&&new URLSearchParams(location.search).has("initial")),failDismiss:false,readFail:false};
await import("./voice-browser-fixture.js");
const original=window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
const kinds=["minutes_generation","diarization","transcription","backup","restore","model","recording_save","recording_recovery","audio_import","voice_registration","voice_matching","speaker_completion","deletion","audio_deletion","model_deletion"];
const outcomes=["succeeded","failed","cancelled","interrupted","attention"];
const time=Date.now();
f.rows=Array.from({length:57},(_,i)=>({id:`history-${String(i).padStart(3,"0")}`,kind:kinds[i%kinds.length],target:`企画会議 ${i+1}`,meetingId:i%kinds.length<3?"meeting-0":null,state:outcomes[i%outcomes.length],acceptedAt:time-i*600000-60000,finishedAt:i%outcomes.length>=3?null:time-i*600000,observedAt:time-i*600000,detail:i%outcomes.length===1?"合成の処理失敗です。":null}));
const sample=(id,modelName,hash)=>({id,modelName,modelHash:hash.repeat(64),extractorVersion:"test",sourceMeetingId:null,sourceAvailable:false,audioAvailable:true,audioStorageState:"ready",startUs:1500000,endUs:4500000,createdAt:0});
window.monVoiceFixture.people=[{id:"9b3b937d-test-person",name:"田中 太郎",version:1,samples:[sample("known","3D-Speaker ERes2Net base","a"),sample("unknown",null,"b")]}];
window.__TAURI_INTERNALS__.invoke=async(command,args={})=>{
  f.calls.push({command,args:structuredClone(args)});
  if(command==="get_model_download_guidance"){
    if(new URLSearchParams(location.search).has("guidance-read-failure"))throw Error("合成の初回案内取得失敗");
    return {pending:f.pending};
  }
  if(command==="record_background_read_failure")return null;
  if(command==="dismiss_model_download_guidance"){
    if(f.failDismiss)throw Error("合成の案内終了保存失敗");
    f.pending=false;localStorage.setItem(key,"done");return {pending:false};
  }
  if(command==="list_job_history"){
    if(f.readFail)throw Error("合成の履歴取得失敗");
    const q=args.query;
    const rows=f.rows.filter(r=>(!q.kind||r.kind===q.kind)&&(!q.state||r.state===q.state)&&(!q.cursor||(r.finishedAt??r.observedAt)<q.cursor.at||(r.finishedAt??r.observedAt)===q.cursor.at&&r.id<q.cursor.id));
    const items=rows.slice(0,q.limit),last=items.at(-1);
    return {items:structuredClone(items),nextCursor:rows.length>q.limit?{at:last.finishedAt??last.observedAt,id:last.id}:null,storageError:null};
  }
  return original(command,args);
};
f.changed=()=>emit("job-history-changed",null);
