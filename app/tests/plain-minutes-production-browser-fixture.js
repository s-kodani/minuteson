// Product main window with synthetic in-memory storage. No native DB/model,
// recording, playback or file writes. Old fixture supplies unrelated commands.
import { installMinutesFixture } from "./minutes-browser-fixture.js";
await installMinutesFixture({ addInitScript: async initialize => initialize(), reload: async () => {} });
const data=window.__mon, copy=value=>structuredClone(value);
const op=n=>`00000000-0000-4000-8000-${String(n).padStart(12,"0")}`;
data.plain={m1:{meetingId:"m1",markdown:"# 点検会議\n\n- 点検日を確認する\n\n人間の保存済み追記\n",lockVersion:1,contentHash:"a".repeat(64),operationId:op(1),updatedAt:1}};
if(new URLSearchParams(location.search).get("empty")==="1")data.plain={};
data.plainMode="success";data.startMode="success";data.plainCalls=[];data.plainRelease=()=>{};
data.exportMode="success";data.exportRecords={};data.exportActive=false;data.exportRelease=()=>{};
const callbacks=new Map(),listeners=new Map();
const bridge=window.__TAURI_INTERNALS__,originalInvoke=bridge.invoke.bind(bridge),originalTransform=bridge.transformCallback.bind(bridge);
bridge.transformCallback=fn=>{const id=originalTransform(fn);callbacks.set(id,fn);return id;};
data.emitPlain=(event,payload="m1")=>{for(const [id,listener] of listeners)if(listener.event===event)callbacks.get(listener.handler)?.({event,id,payload});};
data.completePlain=()=>{
  const job=data.jobs.at(-1);if(!job)throw Error("no job");
  const saved=data.plain[job.meetingId]??null,expected=job.expected;
  if((saved?.lockVersion??null)!==(expected?.lockVersion??null)||(saved?.contentHash??null)!==(expected?.contentHash??null)) {job.state="failed";job.errorCode="合成の人間保存との競合";}
  else {data.plain[job.meetingId]={meetingId:job.meetingId,markdown:"# 生成された議事録\n\n- 点検日を確認する\n",lockVersion:(expected?.lockVersion??0)+1,contentHash:"b".repeat(64),operationId:job.operationId,updatedAt:2};job.state="succeeded";job.completedUnits=job.totalUnits;}
  data.emitPlain("analysis-job-changed");data.emitPlain("minutes-version-changed");
};
bridge.invoke=async(cmd,args={})=>{
  if(cmd==="get_minutes_preferences"&&data.minutesPrefReadFailure)throw Error("合成の全体設定読込み失敗");
  if(cmd==="plugin:event|listen"){const id=await originalInvoke(cmd,args);listeners.set(id,args);return id;}
  if(cmd==="plugin:event|unlisten"){listeners.delete(args.eventId);return originalInvoke(cmd,args);}
  if(cmd==="get_minutes_markdown"){
    data.plainCalls.push({cmd,args:copy(args)});
    if(data.plainReadFailure)throw Error("合成本文の読込み失敗");return copy(data.plain[args.meetingId]??null);
  }
  if(cmd==="save_minutes_markdown"){
    data.plainCalls.push({cmd,args:copy(args)});const r=args.request;
    if(data.plainMode==="held")await new Promise(resolve=>{data.plainRelease=resolve;});
    if(data.plainMode==="failure")throw Error("合成の保存失敗");
    const saved=data.plain[r.meetingId];
    if(saved?.operationId!==r.operationId){
      if((saved?.lockVersion??null)!==(r.expected?.lockVersion??null)||(saved?.contentHash??null)!==(r.expected?.contentHash??null))throw Error("合成の保存競合");
      data.plain[r.meetingId]={meetingId:r.meetingId,markdown:r.markdown,lockVersion:(r.expected?.lockVersion??0)+1,contentHash:"c".repeat(64),operationId:r.operationId,updatedAt:3};
    }else if(saved.markdown!==r.markdown)throw Error("reused operation with wrong body");
    if(data.plainMode==="lost")throw Error("合成の保存成功応答喪失");
    return copy(data.plain[r.meetingId]);
  }
  if(cmd==="get_plain_minutes_export_status"){
    data.plainCalls.push({cmd,args:copy(args)});
    if(data.exportStatusFailure)throw Error("合成の結果読込み失敗");
    return {active:data.exportActive,record:copy(data.exportRecords[args.request.operationId]??null)};
  }
  if(cmd==="export_plain_minutes_markdown"){
    data.plainCalls.push({cmd,args:copy(args)});const r=copy(args.request);
    if(data.exportRecords[r.operationId])return copy(data.exportRecords[r.operationId]);
    if(data.exportMode==="cancel")return null;
    if(data.exportMode==="transport")throw Error("合成の書き出し通信失敗");
    const saved=copy(data.plain[r.meetingId]);
    data.exportActive=true;
    try {
      if(data.exportMode==="held")await new Promise(resolve=>data.exportRelease=resolve);
      const current=data.plain[r.meetingId];
      if(current?.lockVersion!==r.expected.lockVersion||current?.contentHash!==r.expected.contentHash)throw Error("合成の保存先選択中の本文競合");
      const bytes=new TextEncoder().encode(saved.markdown),hash=await crypto.subtle.digest("SHA-256",bytes);
      const failed=data.exportMode==="failure";
      const record={id:r.operationId,meetingId:r.meetingId,minutesRevisionId:null,exportedContentHash:r.expected.contentHash,
        outputContentHash:Array.from(new Uint8Array(hash),b=>b.toString(16).padStart(2,"0")).join(""),outputSizeBytes:bytes.length,
        destinationPath:"/synthetic/議事録.md",state:failed?"failed":"succeeded",errorMessage:failed?"合成の外部書込み失敗":null,createdAt:0,exportedAt:failed?null:1};
      data.exportRecords[r.operationId]=record;
      if(data.exportMode==="lost")throw Error("合成の書き出し成功応答喪失");return copy(record);
    } finally {data.exportActive=false;}
  }
  if(cmd==="start_markdown_minutes_generation"){
    data.plainCalls.push({cmd,args:copy(args)});const r=args.request,saved=data.plain[r.input.meetingId]??null;
    let job=data.jobs.find(j=>j.operationId===r.input.operationId);
    if(!job){
      if((saved?.lockVersion??null)!==(args.expected?.lockVersion??null)||(saved?.contentHash??null)!==(args.expected?.contentHash??null))throw Error("changed Markdown target");
      job={id:`plain-job-${data.jobs.length+1}`,meetingId:r.input.meetingId,kind:"minutes_generation",state:"queued",completedUnits:0,totalUnits:3,errorCode:null,outputAnalysisId:null,outputMinutesId:null,pauseReason:null,specHash:"d".repeat(64),retryOfJobId:null,attempt:1,operationId:r.input.operationId,expected:copy(args.expected)};
      data.jobs.push(job);data.emitPlain("analysis-job-changed");
    }
    if(data.startMode==="lost")throw Error("合成の生成開始応答喪失");return copy(job);
  }
  if(["pause_analysis_job","resume_analysis_job","cancel_analysis_job"].includes(cmd)){
    data.plainCalls.push({cmd,args:copy(args)});const job=data.jobs.find(j=>j.id===args.jobId);if(!job)throw Error("job missing");
    job.state={pause_analysis_job:"paused",resume_analysis_job:"queued",cancel_analysis_job:"cancelled"}[cmd];data.emitPlain("analysis-job-changed");return copy(job);
  }
  if(cmd==="retry_minutes_generation"){
    data.plainCalls.push({cmd,args:copy(args)});const parent=data.jobs.find(j=>j.id===args.request.jobId);if(!parent)throw Error("parent missing");
    const job={...copy(parent),id:`plain-job-${data.jobs.length+1}`,state:"queued",errorCode:null,operationId:args.request.operationId,attempt:parent.attempt+1,retryOfJobId:parent.id};data.jobs.push(job);data.emitPlain("analysis-job-changed");return copy(job);
  }
  if(/^(get_minutes_(versions|preview|draft|evidence_source|transcript_page|exports)|save_minutes_draft|adopt_minutes_candidate|restore_minutes_version|start_minutes_generation|prepare_minutes_)/.test(cmd))throw Error(`obsolete minutes command: ${cmd}`);
  return originalInvoke(cmd,args);
};
await import("../src/main.tsx");
