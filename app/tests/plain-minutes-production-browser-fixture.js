// Product main window with synthetic in-memory storage. No native DB/model,
// recording, playback or file writes. Window fixture supplies unrelated current commands.
import { installPlainMinutesWindowFixture } from "./plain-minutes-window-fixture.js";
await installPlainMinutesWindowFixture({ addInitScript: async initialize => initialize(), reload: async () => {} });
const data=window.__mon, copy=value=>structuredClone(value);
const op=n=>`00000000-0000-4000-8000-${String(n).padStart(12,"0")}`;
data.plain={m1:{meetingId:"m1",markdown:"# 点検会議\n\n- 点検日を確認する\n\n人間の保存済み追記\n",lockVersion:1,contentHash:"a".repeat(64),operationId:op(1),updatedAt:1}};
if(window.__monMarkdownIntervalFixture)data.plain.m1.markdown="# 点検会議\n\n[点検区間](audio:00:00:01.000001-00:00:03.999999)\n\n[範囲外](audio:00:01:00-00:01:01) [逆順](audio:00:00:04-00:00:03) [外部](https://example.com)\n";
data.audioSummary={schemaVersion:1,durationUs:60000000,modes:["mixed"]};
data.intervalMode="success";data.intervalCalls=[];data.intervalRelease=()=>{};
if(new URLSearchParams(location.search).get("empty")==="1")data.plain={};
data.plainMode="success";data.startMode="success";data.plainCalls=[];data.plainReads=[];data.plainRelease=()=>{};
data.exportMode="success";data.exportRecords={};data.exportActive=false;data.exportRelease=()=>{};
data.health = new URLSearchParams(location.search).has("resilience") ? {
  listError: true, draftReadFailure: true, draftDiscarded: false, draftCreated: false, audioIssue: true,
  shared: false, salvageAvailable: false, storageReadCount: 0, modelListingFailure: true,
  holdReason: "保存先へのアクセス権限を確認できません。アクセス権限を戻してください。",
} : null;
const callbacks=new Map(),listeners=new Map();
const bridge=window.__TAURI_INTERNALS__,originalInvoke=bridge.invoke.bind(bridge),originalTransform=bridge.transformCallback.bind(bridge);
bridge.transformCallback=fn=>{const id=originalTransform(fn);callbacks.set(id,fn);return id;};
data.emitPlain=(event,payload="m1")=>{for(const [id,listener] of listeners)if(listener.event===event)callbacks.get(listener.handler)?.({event,id,payload});};
data.completePlain=()=>{
  const job=data.jobs.at(-1);if(!job)throw Error("no job");
  const saved=data.plain[job.meetingId]??null,expected=job.expected;
  if((saved?.lockVersion??null)!==(expected?.lockVersion??null)||(saved?.contentHash??null)!==(expected?.contentHash??null)) {job.state="failed";job.errorCode="合成の人間保存との競合";}
  else {data.plain[job.meetingId]={meetingId:job.meetingId,markdown:"# 生成された議事録\n\n- 点検日を確認する\n",lockVersion:(expected?.lockVersion??0)+1,contentHash:"b".repeat(64),operationId:job.operationId,updatedAt:2};job.state="succeeded";job.completedUnits=job.totalUnits;}
  data.emitPlain("analysis-job-changed", copy(job));
};
bridge.invoke=async(cmd,args={})=>{
  if (["get_minutes_markdown", "get_minutes_source", "get_minutes_preferences", "get_analysis_jobs", "get_meeting_audio_summary"].includes(cmd)) data.plainReads.push(cmd);
  if(data.health){
    if(cmd==="list_meetings" && data.health.listError)throw Error("合成の会議一覧取得失敗");
    if(cmd==="get_meeting_list_diagnostics" && data.health.diagnosticsReadFailure)throw Error("合成の会議の注意事項読込み失敗");
    if(cmd==="get_meeting_list_diagnostics")return { items:[
      ...(data.health.audioIssue ? [{meetingId:"m1",reason:"audio_recovery",message:"音声に確認が必要です"}] : []),
      ...(data.health.meetingIssue ? [{meetingId:"m1",reason:"meeting_row_invalid",message:"一部の会議情報を確認できません"}] : []),
    ] };
    if(cmd==="get_storage_write_state"){
      data.health.storageReadCount++;
      return { readOnly: data.health.shared, reasons: data.health.shared ? [data.health.holdReason] : [], salvageAvailable: data.health.salvageAvailable };
    }
    if(cmd==="get_data_operation")return { ...(await originalInvoke(cmd,args)),modelDeletionPreview:null,modelDeletionReceipt:null };
    if(cmd==="get_model_deletion_items"){
      if(data.health.modelListingFailure)throw Error("合成のモデル削除一覧取得失敗");
      return { items: [], writeBlocked: false, restartRequired: false, reason: null };
    }
    if(cmd==="get_current_analysis" && args.meetingId==="m1" && data.health.transcriptReadMode){
      if(data.health.transcriptReadMode==="absent")return null;
      throw Error("合成の文字起こし本文の読込み失敗");
    }
    if(cmd==="get_transcript_draft" && args.meetingId==="m1"){
      if(data.health.draftReadFailure)throw Error("合成の下書き読込み応答の喪失");
      if(data.health.draftDiscarded&&!data.health.draftCreated)return null;
    }
    if(cmd==="begin_transcript_draft" && data.health.draftDiscarded && args.meetingId==="m1" && args.analysisId==="analysis"){
      if(!data.health.draftCreated){
        data.td={id:"fresh-transcript-draft",meetingId:"m1",baseAnalysisId:"analysis",lockVersion:0,updatedAt:0,payload:{schema_version:4,speaker_names:[],utterances:[]}};
        data.health.draftCreated=true;
      }
      return (await import("./transcript-draft-fixture.js")).stampFixtureDraft(data.td);
    }
    if(cmd==="get_minutes_source" && data.health.draftDiscarded && !data.health.draftCreated && args.meetingId==="m1")return { ...(await originalInvoke(cmd,args)), draft:null };
    if(cmd==="get_meeting_body_for_review"){
      if(args.part!=="minutes")return data.health.transcriptReview ?? null;
      return data.health.minutesReviewUnavailable ? null : {text:copy(data.plain[args.meetingId]?.markdown??""),complete:true};
    }
  }
  if(cmd==="get_ready_analysis_flows")return [];
  if(cmd==="get_meeting_audio_summary"){
    data.intervalCalls.push({cmd,args:copy(args)});
    if(data.intervalSummaryFailure)throw Error("合成音声情報の読込み失敗");return copy(data.audioSummary);
  }
  if(cmd==="prepare_meeting_playback"||cmd==="prepare_markdown_interval_playback"){
    data.intervalCalls.push({cmd,args:copy(args)});
    // Assign identity before awaiting: late replies must not reuse a newer token.
    const intervalSequence=data.intervalCalls.filter(c=>c.cmd===cmd).length;
    if(data.intervalMode==="held")await new Promise(resolve=>data.intervalRelease=resolve);
    if(data.intervalMode==="failure")throw Error("合成の音声検証失敗");
    const r=args.request;
    const start=cmd==="prepare_meeting_playback"?0:r.startUs,end=cmd==="prepare_meeting_playback"?data.audioSummary.durationUs:r.endUs;
    const first=Math.ceil(start*16000/1000000),last=Math.ceil(end*16000/1000000);
    const token=`interval-${intervalSequence}`;
    data.playbacks[token]={totalFrames:last-first,sampleRate:16000};
    return {token,sampleRate:16000,channels:1,totalFrames:last-first,startUs:start,endUs:end,trackKind:r.mode,alignment:"unverified",
      ...(data.intervalMode==="gap"?{missingRanges:[{trackKind:"mic",startUs:start,endUs:2000000,startFrame:0,endFrame:32000-first,reasonCode:"capture_timestamp_gap"}]}:{}),
      ...(data.intervalMode==="wrong"?{endUs:end+1}:{})};
  }
  if(cmd==="read_playback_chunk"||cmd==="close_playback"||cmd==="cancel_playback_preparation")data.intervalCalls.push({cmd,args:copy(args)});
  if(cmd==="get_minutes_preferences"&&data.minutesPrefReadFailure)throw Error("合成の全体設定読込み失敗");
  if(data.minutesInputsReadFailure && ["get_minutes_source", "get_analysis_jobs"].includes(cmd))throw Error("合成の議事録生成情報の読込み失敗");
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
      const record={id:r.operationId,meetingId:r.meetingId,exportedContentHash:r.expected.contentHash,
        outputContentHash:Array.from(new Uint8Array(hash),b=>b.toString(16).padStart(2,"0")).join(""),outputSizeBytes:bytes.length,
        destinationPath:"/synthetic/議事録.md",state:failed?"failed":"succeeded",errorMessage:failed?"合成の外部書込み失敗":null,createdAt:0,exportedAt:failed?null:1};
      data.exportRecords[r.operationId]=record;
      if(data.exportMode==="lost")throw Error("合成の書き出し成功応答喪失");return copy(record);
    } finally {data.exportActive=false;}
  }
  if(cmd==="start_markdown_minutes_generation"){
    if(data.startMode==="held")await new Promise(resolve=>{data.startRelease=resolve;});
    data.plainCalls.push({cmd,args:copy(args)});const r=args.request,saved=data.plain[r.input.meetingId]??null;
    let job=data.jobs.find(j=>j.operationId===r.input.operationId);
    if(!job){
      if((saved?.lockVersion??null)!==(args.expected?.lockVersion??null)||(saved?.contentHash??null)!==(args.expected?.contentHash??null))throw Error("changed Markdown target");
      job={id:`plain-job-${data.jobs.length+1}`,meetingId:r.input.meetingId,kind:"minutes_generation",state:"queued",completedUnits:0,totalUnits:3,errorCode:null,outputAnalysisId:null,pauseReason:null,specHash:"d".repeat(64),retryOfJobId:null,attempt:1,operationId:r.input.operationId,expected:copy(args.expected)};
      data.jobs.push(job);data.emitPlain("analysis-job-changed", copy(job));
    }
    if(data.startMode==="lost")throw Error("合成の生成開始応答喪失");return copy(job);
  }
  if(["pause_analysis_job","resume_analysis_job","cancel_analysis_job"].includes(cmd)){
    data.plainCalls.push({cmd,args:copy(args)});const job=data.jobs.find(j=>j.id===args.jobId);if(!job)throw Error("job missing");
    job.state={pause_analysis_job:"paused",resume_analysis_job:"queued",cancel_analysis_job:"cancelled"}[cmd];data.emitPlain("analysis-job-changed", copy(job));return copy(job);
  }
  if(cmd==="retry_minutes_generation")throw Error("removed failure retry command");
  if(/^(get_minutes_(versions|preview|draft|evidence_source|transcript_page|exports)|save_minutes_draft|adopt_minutes_candidate|restore_minutes_version|start_minutes_generation|prepare_minutes_)/.test(cmd))throw Error(`obsolete minutes command: ${cmd}`);
  return originalInvoke(cmd,args);
};
await import("../src/bootstrap.tsx");
