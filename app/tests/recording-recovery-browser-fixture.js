// Synthetic IPC only. Never opens native storage or real recordings.
import {mockIPC} from "./startup-ready-mocks.ts";
import {emit} from "@tauri-apps/api/event";
window.skipDataFixtureMain=true;
const {dataSaveIPC}=await import("./data-save-browser-fixture.js");
const f=window.dataFixture;
const ids=["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"];
f.meetings=ids.map((id,i)=>({...f.meetings[0],id,title:i?"回復待ちの会議":"回復中の会議",lifecycle:"recording",sourceType:"recording"}));
Object.assign(f.snapshot,{deletionPreview:null,deletionReceipt:null,modelDeletionPreview:null,modelDeletionReceipt:null,audioDeletionPreview:null,audioDeletionReceipt:null});
const initial={runId:"synthetic-recovery",revision:1,phase:"running",error:null,items:ids.map((meetingId,i)=>({meetingId,sessionId:`session-${i}`,meetingTitle:f.meetings[i].title,phase:i?"queued":"running",startedAtMs:i?null:Date.now()-5000,error:null}))};
const r=window.recordingRecoveryFixture={snapshot:JSON.parse(sessionStorage.getItem("recovery-fixture")??"null")??initial,fail:false,stale:false,holds:[],hold:false,calls:[],
  change:async patch=>{r.snapshot={...r.snapshot,...patch,revision:r.snapshot.revision+1};sessionStorage.setItem("recovery-fixture",JSON.stringify(r.snapshot));await emit("recording-recovery-changed",null);},
  finish:async()=>{for(const m of f.meetings)m.lifecycle="recorded";r.snapshot={...r.snapshot,revision:r.snapshot.revision+1,phase:"completed",items:r.snapshot.items.map(i=>({...i,phase:"succeeded"}))};sessionStorage.setItem("recovery-fixture",JSON.stringify(r.snapshot));},
};
mockIPC(async(command,args)=>{
  r.calls.push(command);
  if(command==="get_recording_recovery") {
    if(r.fail)throw Error("合成状態取得失敗");
    if(r.hold)return new Promise(resolve=>r.holds.push(()=>resolve(structuredClone(initial))));
    return structuredClone(r.stale?initial:r.snapshot);
  }
  if(command==="get_application_close_activity")return {analysisBusy:false,modelPreparationBusy:false,recordingRecoveryBusy:r.snapshot.phase==="running"};
  if(command==="get_meeting_deletion_availability")return args.meetingIds.map(meetingId=>{
    const active=r.snapshot.items.some(i=>i.meetingId===meetingId&&i.phase==="running");
    return {meetingId,available:!active,blockers:active?[{kind:"recording",reason:"音声保存中",jobId:null}]:[]};
  });
  if(command==="get_deletion_items")return {items:[],deletingMeetingIds:[],writeBlocked:false,restartRequired:false,reason:null};
  if(["get_model_deletion_items","get_audio_deletion_items"].includes(command))return {items:[],writeBlocked:false,restartRequired:false,reason:null};
  return dataSaveIPC(command,args);
},{shouldMockEvents:true});
await import("../src/main.tsx");
