// Synthetic IPC around the production React app; no real audio, DB or model.
import {emit} from "@tauri-apps/api/event";
await import("./browser-fixture.js");
const data=window.monBrowserFixture;
const state=window.monVoiceFixture={people:[],matches:[],requests:[],loseResponse:false,failStart:false,supported:true,failPeople:false,failSample:false,holdSample:false,pendingSamples:[],corruptSample:null,contextRevision:0,candidates:[{speakerId:"speaker-a",trackId:"microphone",startUs:0,endUs:15e6,utteranceId:"utterance-a",text:"登録用の発言"}]};
state.emitChange=()=>emit("voice-identity-changed","meeting-0");
const bridge=window.__TAURI_INTERNALS__,original=bridge.invoke.bind(bridge);
const copy=v=>structuredClone(v);
bridge.invoke=async(command,args={})=>{
  const analysis=await original("get_current_analysis",{meetingId:"meeting-0"});
  if(command==="get_analysis_notification_jobs"){const result=await original(command,args);return result.filter(j=>!j.kind.startsWith("voice_"));}
  if(command==="prepare_markdown_interval_playback"){
    data.calls.push({command,args:copy(args)});const r=args.request;
    const info={token:"voice-preview",sampleRate:16000,channels:1,totalFrames:Math.ceil(r.endUs*16000/1e6)-Math.ceil(r.startUs*16000/1e6),startUs:r.startUs,endUs:r.endUs,trackKind:r.mode,alignment:"unverified"};data.meetingAudio.sessions[info.token]=info;return info;
  }
  if(command==="get_voice_people"){if(state.failPeople)throw Error("合成のボイス一覧取得失敗");return copy(state.people);}
  if(command==="prepare_voice_sample_playback"){
    data.calls.push({command,args:copy(args)});
    const entry=data.preparation.strict?data.preparation.reservations[args.preparationId]:null;
    if(data.preparation.strict&&(!entry||entry.claimed||entry.cancelled))throw Error("未登録の音声準備");
    if(entry){entry.claimed=true;entry.reject=()=>{};}
    if(state.holdSample)await new Promise((resolve,reject)=>{state.pendingSamples.push({release:resolve});if(entry)entry.reject=reject;});
    if(state.failSample)throw Error("合成の登録音声再生失敗");
    const sample=state.people.flatMap(p=>p.samples).find(s=>s.id===args.sampleId);
    if(!sample||!sample.sourceAvailable)throw Error("登録元の音声がありません");
    if(entry?.cancelled)throw Error("取消済みの音声準備");
    const info={token:crypto.randomUUID(),sampleRate:16000,channels:1,totalFrames:Math.ceil(sample.endUs*16000/1e6)-Math.floor(sample.startUs*16000/1e6),startUs:sample.startUs,endUs:sample.endUs,trackKind:"processed",alignment:"unverified",...state.corruptSample};
    data.meetingAudio.sessions[info.token]=info;if(entry)entry.token=info.token;return info;
  }
  if(command==="get_voice_context"){
    data.calls.push({command,args:copy(args)});
    return {supported:state.supported,analysisId:analysis.id,workingHash:state.contextRevision.toString(16).padStart(64,"d"),bookHash:"e".repeat(64),people:copy(state.people),matches:copy(state.matches),windows:[{speakerId:"speaker-a",trackId:"microphone",startUs:0,endUs:15e6}],registrationCandidates:copy(state.candidates)};
  }
  if(command==="start_voice_job"){
    if(state.failStart)return {state:"rejected",message:"合成の登録失敗"};
    let job=data.jobs.find(j=>j.operationId===args.request.operationId);
    if(!job){state.requests.push(copy(args.request));job={id:crypto.randomUUID(),meetingId:"meeting-0",meetingTitle:"会議A",kind:args.request.speakerId?"voice_registration":"voice_matching",state:"queued",operationId:args.request.operationId,inputHash:"a".repeat(64),specHash:"b".repeat(64),completedUnits:0,totalUnits:1,errorCode:null,outputAnalysisId:null,outputMinutesId:null,pauseReason:null,retryOfJobId:null,attempt:1,createdAt:Date.now()};data.jobs.push(job);}
    if(state.loseResponse){state.loseResponse=false;throw Error("合成の受付応答欠落");}return {state:"accepted",job:copy(job)};
  }
  if(command==="manage_voice_person"){
    const p=state.people.find(p=>p.id===args.personId);if(!p||p.version!==args.expectedVersion)throw Error("人物登録が変更されています");
    if(args.sampleId)p.samples=p.samples.filter(s=>s.id!==args.sampleId);else if(args.name)p.name=args.name;else state.people=state.people.filter(q=>q.id!==p.id);p.version++;return;
  }
  if(command==="decide_voice_match"){
    const match=state.matches.find(m=>m.speakerId===args.speakerId);
    const person=state.people.find(p=>p.id===args.personId);
    if(args.personId&&!person)throw Error("人物登録がありません");
    let draft=await original("get_transcript_draft",{meetingId:"meeting-0"});if(!draft)draft=await original("begin_transcript_draft",{meetingId:"meeting-0",analysisId:analysis.id});
    draft.payload.speaker_names=draft.payload.speaker_names.filter(n=>n.speaker_id!==args.speakerId);draft.payload.speaker_names.push({speaker_id:args.speakerId,display_name:person?.name??null});
    await original("save_transcript_draft",{draftId:draft.id,expectedLockVersion:draft.lockVersion,payload:draft.payload});
    Object.assign(match,{state:person?"confirmed":"dismissed",personId:person?.id??null,nameSnapshot:person?.name??match.nameSnapshot});return;
  }
  return original(command,args);
};
state.complete=()=>{
  const job=data.jobs.find(j=>j.state==="queued"&&(j.kind==="voice_registration"||j.kind==="voice_matching"));
  if(!job)throw Error("no voice job");
  if(job.kind==="voice_registration")state.people.push({id:"person-a",name:state.requests.at(-1).personName??"登録者",version:1,samples:[{id:"sample-a",modelHash:"a".repeat(64),extractorVersion:"test",sourceMeetingId:"meeting-0",sourceAvailable:true,startUs:0,endUs:15e6,createdAt:0}]});
  else state.matches=[{id:"match-a",speakerId:"speaker-a",personId:"person-a",nameSnapshot:state.people[0].name,state:"candidate",similarity:0.98,modelHash:"a".repeat(64)}];
  job.state="succeeded";job.completedUnits=1;
};
