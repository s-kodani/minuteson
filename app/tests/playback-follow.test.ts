import test from "node:test";
import assert from "node:assert/strict";
import { playingRows, playbackTimeline, type PlaybackTimeline } from "../src/playback-follow.ts";

const emphasizedRow=(...args:Parameters<typeof playingRows>)=>playingRows(...args)[0]??null;
const timeline:PlaybackTimeline={owner:"a",session:"a:1",meetingId:"m",analysisId:"analysis",utteranceId:"u",scope:"utterance",timeUs:1000000,playing:true,trackKind:"mic"};
const rows=[{id:"u",trackId:"native-mic-id",startUs:0,endUs:2000000},{id:"second",trackId:"native-system-id",startUs:3000000,endUs:4000000}];
const tracks=new Map([["native-mic-id","mic"],["native-system-id","system"]]);
test("emphasis only addresses playing timeline, same meeting and exact current utterance analysis",()=>{
  assert.equal(emphasizedRow(rows,timeline,"m","analysis"),0);
  for(const value of [null,{...timeline,playing:false},{...timeline,meetingId:"other"},{...timeline,analysisId:"frozen-minutes"},{...timeline,timeUs:2000000},{...timeline,timeUs:4000000},{...timeline,utteranceId:"other"}])assert.equal(emphasizedRow(rows,value,"m","analysis"),null);
});
test("whole meeting emphasizes time within filtered rows without inventing speech in gaps or switching filters",()=>{
  const full={...timeline,scope:"meeting" as const,analysisId:null,utteranceId:null,timeUs:3500000,trackKind:"both"};
  assert.equal(emphasizedRow(rows,full,"m","analysis",tracks),1);
  assert.equal(emphasizedRow(rows.slice(0,1),full,"m","analysis",tracks),null);
  assert.equal(emphasizedRow(rows,{...full,timeUs:2500000},"m","analysis",tracks),null);
  assert.equal(emphasizedRow(rows,{...full,timeUs:20000000000},"m","analysis",tracks),null);
});
test("whole-meeting emphasis respects the native source kind and highlights every actual overlapping row",()=>{
  const overlap=[rows[0],{id:"simultaneous",trackId:"native-system-id",startUs:500000,endUs:1500000},
    {id:"import",trackId:"import-id",startUs:0,endUs:2000000},
    {id:"unknown",trackId:"mic-looking-but-unregistered",startUs:0,endUs:2000000},
    {id:"missing",startUs:0,endUs:2000000}];
  const kinds=new Map([...tracks,["import-id","imported"]]);
  const full={...timeline,scope:"meeting" as const,analysisId:null,utteranceId:null};
  for(const [trackKind,expected] of [["mic",[0]],["system",[1]],["both",[0,1]],["imported",[2]],["processed",[]],["mixed",[0,1,2]]] as const){
    assert.deepEqual(playingRows(overlap,{...full,trackKind},"m","analysis",kinds),expected);
    assert.equal(emphasizedRow(overlap,{...full,trackKind},"m","analysis",kinds),expected[0]??null);
  }
  assert.deepEqual(playingRows(overlap,{...full,trackKind:"both"},"m","analysis",null),[]);
  for(const patch of [{playing:false},{meetingId:"other"},{timeUs:NaN},{timeUs:-1},{timeUs:Infinity}]){
    assert.deepEqual(playingRows(overlap,{...full,trackKind:"both",...patch},"m","analysis",kinds),[]);
  }
  assert.deepEqual(playingRows([{id:"bad",trackId:"native-mic-id",startUs:0,endUs:NaN}],full,"m","analysis",kinds),[]);
});
test("split utterance emphasizes only its source interval and does not invent a split position",()=>{
  const split=[{id:"left",sourceUtteranceId:"u",startUs:0,endUs:1000000},{id:"right",sourceUtteranceId:"u",startUs:1000000,endUs:2000000}];
  assert.equal(emphasizedRow(split,timeline,"m","analysis"),1);
  assert.equal(emphasizedRow(split,{...timeline,timeUs:500000},"m","analysis"),0);
});
test("shared timeline publishes changed state only; late old-owner release never clears a newer source",()=>{
  let calls=0;const stop=playbackTimeline.subscribe(()=>calls++);
  playbackTimeline.publish(timeline);assert.equal(calls,1);
  playbackTimeline.publish({...timeline});assert.equal(calls,1);
  playbackTimeline.publish({...timeline,owner:"b",session:"b:1"});assert.equal(calls,2);
  playbackTimeline.release("a");assert.equal(playbackTimeline.snapshot()?.owner,"b");assert.equal(calls,2);
  playbackTimeline.publish({...timeline,timeUs:NaN});assert.equal(calls,2);
  playbackTimeline.release("b");assert.equal(playbackTimeline.snapshot(),null);assert.equal(calls,3);stop();
});

test('mixed analysis utterances keep meeting time while listening to captured sources',()=>{
 const mixed=[{id:'mix-speech',trackId:'mixed-native-id',startUs:500000,endUs:1500000}];
 const kinds=new Map([['mixed-native-id','processed']]);
 for(const trackKind of ['mic','system','both','imported','processed','mixed']) {
  assert.deepEqual(playingRows(mixed,{...timeline,scope:'meeting',analysisId:null,utteranceId:null,trackKind},'m','analysis',kinds),[0]);
 }
 assert.deepEqual(playingRows(mixed,{...timeline,scope:'meeting',trackKind:'foreign'},'m','analysis',kinds),[]);
});

test('canonical mixed playback requires a known track and actual speech at the current time',()=>{
 const full={...timeline,scope:'meeting' as const,analysisId:null,utteranceId:null,trackKind:'mixed'};
 assert.deepEqual(playingRows(rows,full,'m','analysis',tracks),[0]);
 assert.deepEqual(playingRows(rows,{...full,timeUs:3500000},'m','analysis',tracks),[1]);
 for(const patch of [{timeUs:2500000},{timeUs:4000000},{meetingId:'other'},{playing:false}]) {
  assert.deepEqual(playingRows(rows,{...full,...patch},'m','analysis',tracks),[]);
 }
 assert.deepEqual(playingRows(rows,full,'m','analysis',null),[]);
});
