import test from "node:test";
import assert from "node:assert/strict";
import { diarizationConfigured,fixedTracks,fixedDiarizationInput,checkedSpeakerReviews,preciseTime,type DiarizationEngine,type SpeakerReview } from "../src/diarization-state.ts";
import { canRetryJob,type VersionState,type AnalysisJob } from "../src/analysis-state.ts";
import { playbackStartFrame } from "../src/playback.ts";
import { DraftSaveQueue } from "../src/draft-save.ts";
const engine:DiarizationEngine={executablePath:"/cli",segmentationModelPath:"/seg",embeddingModelPath:"/emb",runtimeLibraryPaths:[],threads:4,clusterThresholdMilli:500,windowShiftMilli:100,minDurationOnMs:0,minDurationOffMs:0};
test("diarization requires all fixed model paths and bounded native settings",()=>{
  assert.ok(diarizationConfigured(engine));
  for(const patch of [{segmentationModelPath:""},{embeddingModelPath:"relative"},{runtimeLibraryPaths:[""]},{threads:0},{threads:1.1},{clusterThresholdMilli:2001},{windowShiftMilli:0},{minDurationOnMs:60001},{executablePath:"/cli\n"},{runtimeLibraryPaths:Array(17).fill("/runtime")}])assert.equal(diarizationConfigured({...engine,...patch}),false);
});
test("count applies explicitly to each track and cannot silently drop a blank or fractional value",()=>{
  const tracks=[{id:"mic",kind:"mic",startUs:0,endUs:1000},{id:"system",kind:"system",startUs:0,endUs:1000}];
  assert.deepEqual(fixedTracks(tracks,{mic:"3"}),[{trackId:"mic",numSpeakers:3},{trackId:"system",numSpeakers:null}]);
  for(const count of ["","0","101","1.5","-1","NaN"])assert.throws(()=>fixedTracks(tracks,{mic:count}));
  assert.throws(()=>fixedTracks([],{}));assert.throws(()=>fixedTracks([tracks[0],tracks[0]],{}));
});
test("start uses only a committed current version and the latest saved draft stamp",()=>{
  const state:VersionState={meetingLockVersion:7,currentId:"a",previousId:null,draft:{id:"d",lockVersion:4},versions:[{id:"a",revisionNo:1,contentHash:"hash",publicationState:"committed",utteranceCount:1,createdAt:0,profile:{},config:{}}]};
  assert.deepEqual(fixedDiarizationInput(state,"m","op"),{meetingId:"m",operationId:"op",expectedAnalysisId:"a",expectedContentHash:"hash",expectedMeetingLockVersion:7,expectedDraft:state.draft});
  assert.throws(()=>fixedDiarizationInput({...state,currentId:null},"m","op"));
  assert.throws(()=>fixedDiarizationInput({...state,versions:[{...state.versions[0],publicationState:"candidate"}]},"m","op"));
});
test("review refuses another version hash or utterance list before display",()=>{
  const r:SpeakerReview={utteranceId:"u",trackId:"mic",estimatedState:"needsSplit",manualAssignment:true,segmentCount:2,overlapCount:0,page:0,pageCount:1,segments:[],overlaps:[]};
  const response={analysisId:"a",contentHash:"h",reviews:[r]};
  assert.equal(checkedSpeakerReviews(response,"a","h",["u"])[0].estimatedState,"needsSplit");
  assert.throws(()=>checkedSpeakerReviews(response,"b","h",["u"]));assert.throws(()=>checkedSpeakerReviews(response,"a","changed",["u"]));
  assert.throws(()=>checkedSpeakerReviews({...response,reviews:[r,r]},"a","h",["u","v"]));
  assert.throws(()=>checkedSpeakerReviews(response,"a","h",["v"]));
  assert.equal(preciseTime(3723123456),"01:02:03.123");
});
test("speaker interval audio uses the utterance origin, not meeting zero",()=>{
  const info={token:"t",sampleRate:16000,totalFrames:32000,channels:1,startUs:4000000,endUs:6000000,trackKind:"mic",alignment:"unverified"};
  assert.equal(playbackStartFrame(info,4500000),8000);assert.equal(playbackStartFrame(info,5999999),31999);
  for(const us of [0,6000000,NaN,4500000.5])assert.throws(()=>playbackStartFrame(info,us));
});
test("diarization retry is not transcription retry and excludes unfinished children",()=>{
  const j:AnalysisJob={id:"j",kind:"diarization",state:"failed",completedUnits:1,totalUnits:2,errorCode:null,outputAnalysisId:null,pauseReason:null,specHash:"h",retryOfJobId:null,attempt:1};
  assert.ok(canRetryJob(j,[j],"diarization"));assert.equal(canRetryJob(j,[j]),false);
  for(const state of ["queued","running","pausing","paused","interrupted"])assert.equal(canRetryJob(j,[j,{...j,id:"child",retryOfJobId:"j",state}],"diarization"),false);
});
test("a failed profile save preserves input and requires explicit retry",async()=>{
  let fail=true;const saved:unknown[]=[];
  const queue=new DraftSaveQueue(async(id:string,lockVersion:number,payload:{standard:DiarizationEngine})=>{saved.push(payload);if(fail)throw Error("disk");return{id,lockVersion:lockVersion+1,payload};},structuredClone);
  queue.load({id:"preferences",lockVersion:0,payload:{standard:engine}});
  queue.edit(p=>({standard:{...p.standard,threads:2}}));await assert.rejects(queue.flush());
  assert.equal(queue.snapshot().payload?.standard.threads,2);await assert.rejects(queue.flush());
  fail=false;await queue.retry();assert.equal(queue.snapshot().dirty,false);assert.equal(saved.length,2);
});
