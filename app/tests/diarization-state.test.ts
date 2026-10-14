import test from "node:test";
import assert from "node:assert/strict";
import { diarizationConfigured,type DiarizationEngine } from "../src/diarization-state.ts";
import { playbackStartFrame } from "../src/playback.ts";
import { DraftSaveQueue } from "../src/draft-save.ts";
const engine:DiarizationEngine={executablePath:"/cli",segmentationModelPath:"/seg",embeddingModelPath:"/emb",runtimeLibraryPaths:[],threads:4,clusterThresholdMilli:500,windowShiftMilli:100,minDurationOnMs:0,minDurationOffMs:0};
test("diarization requires all fixed model paths and bounded native settings",()=>{
  assert.ok(diarizationConfigured(engine));
  for(const patch of [{segmentationModelPath:""},{embeddingModelPath:"relative"},{runtimeLibraryPaths:[""]},{threads:0},{threads:1.1},{clusterThresholdMilli:2001},{windowShiftMilli:0},{minDurationOnMs:60001},{executablePath:"/cli\n"},{runtimeLibraryPaths:Array(17).fill("/runtime")}])assert.equal(diarizationConfigured({...engine,...patch}),false);
});
test("speaker interval audio uses the utterance origin, not meeting zero",()=>{
  const info={token:"t",sampleRate:16000,totalFrames:32000,channels:1,startUs:4000000,endUs:6000000,trackKind:"mic",alignment:"unverified"};
  assert.equal(playbackStartFrame(info,4500000),8000);assert.equal(playbackStartFrame(info,5999999),31999);
  for(const us of [0,6000000,NaN,4500000.5])assert.throws(()=>playbackStartFrame(info,us));
});
test("a failed profile save preserves input and requires explicit retry",async()=>{
  let fail=true;const saved:unknown[]=[];
  const queue=new DraftSaveQueue(async(id:string,lockVersion:number,payload:{standard:DiarizationEngine})=>{saved.push(payload);if(fail)throw Error("disk");return{id,lockVersion:lockVersion+1,payload};},structuredClone);
  queue.load({id:"preferences",lockVersion:0,payload:{standard:engine}});
  queue.edit(p=>({standard:{...p.standard,threads:2}}));await assert.rejects(queue.flush());
  assert.equal(queue.snapshot().payload?.standard.threads,2);await assert.rejects(queue.flush());
  fail=false;await queue.retry();assert.equal(queue.snapshot().dirty,false);assert.equal(saved.length,2);
});
