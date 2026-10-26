import test from "node:test";
import assert from "node:assert/strict";
import { validateAudioSummary, markdownIntervalRequest, validateMarkdownPlayback } from "../src/minutes-audio-state.ts";
import { ChunkPlayer, type PlaybackClock, type PlaybackInfo } from "../src/playback.ts";
const summary={schemaVersion:1 as const,durationUs:1000000,modes:["mixed" as const]};
const request=markdownIntervalRequest("m",summary,{startUs:250001,endUs:300000});
const info:PlaybackInfo={token:"interval",sampleRate:48000,channels:1,startUs:250001,endUs:300000,totalFrames:2399,trackKind:"mixed",alignment:"unverified"};
test("Markdown intervals use the one canonical source",()=>{
  assert.equal(request.mode,"mixed");assert.doesNotThrow(()=>validateMarkdownPlayback(info,request));
  assert.deepEqual(validateAudioSummary(summary),summary);
});
test("unsafe summary modes and timelines are refused",()=>{
  for(const patch of [{durationUs:0},{durationUs:NaN},{schemaVersion:2},{modes:["mic"]},{modes:["mixed","mixed"]}]) {
    assert.throws(()=>validateAudioSummary({...summary,...patch} as never));
  }
  assert.throws(()=>markdownIntervalRequest("m",summary,{startUs:0,endUs:1000001}));
});
test("a response must match the exact interval, frame count and source",()=>{
  for(const patch of [{startUs:250000},{endUs:300001},{totalFrames:2400},{sampleRate:16000},{channels:2},{trackKind:"mic"},{alignment:"verified"}]) {
    assert.throws(()=>validateMarkdownPlayback({...info,...patch},request));
  }
  assert.throws(()=>validateMarkdownPlayback({...info,missingRanges:[{trackKind:"mic",startUs:250001,endUs:300000,startFrame:0,endFrame:2399,reasonCode:"missing"}]},request));
});
test("interval playback starts at its first PCM frame even when that PCM is silence",async()=>{
  const reads:number[]=[];
  const clock:PlaybackClock={now:()=>0,resume:async()=>{},schedule:()=>()=>{}};
  const player=new ChunkPlayer(clock,async(_,frame)=>{reads.push(frame);throw Error("synthetic read failure");},()=>{});
  player.load(info);await player.play();assert.deepEqual(reads,[0]);assert.equal(player.snapshot().playing,false);
  assert.match(player.snapshot().error!,/synthetic read failure/);player.clear();
});
