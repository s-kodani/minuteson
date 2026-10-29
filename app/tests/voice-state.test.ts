import {test} from "node:test";
import assert from "node:assert/strict";
import {voiceStartRequest,registrationName,voiceMatchLabel,initialVoiceSelection,selectedVoiceWindows,voiceWindowKey,type VoiceContext,type VoiceCandidate} from "../src/voice-state.ts";
const context:VoiceContext={supported:true,analysisId:"a",workingHash:"b".repeat(64),bookHash:"c".repeat(64),people:[{id:"p",name:"田中",version:1,samples:[]}],matches:[],registrationCandidates:[],windows:[{speakerId:"s",trackId:"t",startUs:0,endUs:2e6}]};
test("enrollment explicitly binds the selected speaker, person and current editor/registry",()=>{
  assert.deepEqual(voiceStartRequest("m",context,"s",null," 田中 ","op"),{meetingId:"m",operationId:"op",analysisId:"a",workingHash:context.workingHash,bookHash:context.bookHash,speakerId:"s",personId:null,personName:"田中"});
  assert.equal(voiceStartRequest("m",context,"s","p","ignored","op").personName,null);
  assert.equal(voiceStartRequest("m",context,null,null,"ignored","op").speakerId,null);
});
test("unsupported platforms, stale people and ineligible mixed voices do not start enrollment",()=>{
  assert.throws(()=>voiceStartRequest("m",{...context,supported:false},"s",null,"田中","op"),/この環境/);
  assert.throws(()=>voiceStartRequest("m",context,"missing",null,"田中","op"),/単独/);
  assert.throws(()=>voiceStartRequest("m",context,"s","missing","","op"),/人物/);
  for(const name of [" ","a".repeat(201),"bad\nname"]){assert.throws(()=>registrationName(name));}
  assert.equal(voiceMatchLabel("candidate"),"候補");assert.equal(voiceMatchLabel("automatic"),"自動識別");
});

test("manual registration binds only selected candidates without using matching windows",()=>{
  const candidates:VoiceCandidate[]=Array.from({length:4},(_,i)=>({speakerId:"manual",trackId:"t",startUs:i*10e6,endUs:i*10e6+(i===2?5e6:2e6),utteranceId:`u${i}`,text:`発言${i}`}));
  const c={...context,registrationCandidates:candidates,windows:[]};
  const keys=initialVoiceSelection(candidates);
  assert.deepEqual(keys,[voiceWindowKey(candidates[2]),voiceWindowKey(candidates[0])]);
  const request=voiceStartRequest("m",c,"manual",null,"田中","op",[keys[0]]);
  assert.deepEqual(request.selectedWindows,[{speakerId:"manual",trackId:"t",startUs:20e6,endUs:25e6}]);
  assert.equal(Object.hasOwn(request.selectedWindows![0],"text"),false);
  assert.equal(Object.hasOwn(voiceStartRequest("m",c,null,null,"","op"),"selectedWindows"),false);
  for(const selected of [[],[keys[0],keys[0]],[...keys,voiceWindowKey(candidates[1])],["stale"]]){
    assert.throws(()=>selectedVoiceWindows(candidates,"manual",selected));
  }
  assert.throws(()=>selectedVoiceWindows(candidates,"another",keys));
  assert.throws(()=>voiceStartRequest("m",{...c,registrationCandidates:[]},"manual",null,"田中","op",keys));
});

test("voice completion receipts bind the exact job specification without fabricating an analysis revision",async()=>{
  const {verifiedJobOutput,JobNotificationTracker}=await import("../src/job-notification-state.ts");
  const queued={id:"voice-job",meetingId:"meeting",meetingTitle:"会議",kind:"voice_registration" as const,state:"queued",operationId:"operation",inputHash:"a".repeat(64),specHash:"b".repeat(64),completedUnits:0,totalUnits:1,errorCode:null,outputAnalysisId:null,outputMinutesId:null,pauseReason:null,retryOfJobId:null,attempt:1,createdAt:100,updatedAt:100};
  const job={...queued,state:"succeeded",completedUnits:1,updatedAt:200};
  const proof={job,contentHash:job.specHash,publicationState:"voice"};
  assert.equal(verifiedJobOutput(job,proof),"voice");
  assert.throws(()=>verifiedJobOutput(job,{...proof,contentHash:"c".repeat(64)}),/完了記録/);
  const tracker=new JobNotificationTracker(0);tracker.ingest([queued]);assert.equal(tracker.ingest([job]).pending.length,1);assert.match(tracker.confirmOutput(job,"voice")!.message,/声の登録/);
});
