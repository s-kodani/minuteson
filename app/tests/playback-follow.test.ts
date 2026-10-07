import test from "node:test";
import assert from "node:assert/strict";
import { followingRow, followingScrollTop, playingRows, playbackTimeline, type PlaybackTimeline } from "../src/playback-follow.ts";

const timeline:PlaybackTimeline={owner:"a",session:"a:1",meetingId:"m",analysisId:"analysis",utteranceId:"u",scope:"utterance",timeUs:1000000,playing:true,trackKind:"mic"};
const rows=[{id:"u",trackId:"native-mic-id",startUs:0,endUs:2000000},{id:"second",trackId:"native-system-id",startUs:3000000,endUs:4000000}];
const tracks=new Map([["native-mic-id","mic"],["native-system-id","system"]]);
test("follow only addresses playing timeline, same meeting and exact current utterance analysis",()=>{
  assert.equal(followingRow(rows,timeline,"m","analysis"),0);
  for(const value of [null,{...timeline,playing:false},{...timeline,meetingId:"other"},{...timeline,analysisId:"frozen-minutes"},{...timeline,timeUs:2000000},{...timeline,timeUs:4000000},{...timeline,utteranceId:"other"}])assert.equal(followingRow(rows,value,"m","analysis"),null);
});
test("whole meeting follows time within filtered rows without inventing speech in gaps or switching filters",()=>{
  const full={...timeline,scope:"meeting" as const,analysisId:null,utteranceId:null,timeUs:3500000,trackKind:"both"};
  assert.equal(followingRow(rows,full,"m","analysis",tracks),1);
  assert.equal(followingRow(rows.slice(0,1),full,"m","analysis",tracks),null);
  assert.equal(followingRow(rows,{...full,timeUs:2500000},"m","analysis",tracks),null);
  assert.equal(followingRow(rows,{...full,timeUs:20000000000},"m","analysis",tracks),null);
});
test("whole-meeting emphasis respects the native source kind and highlights every actual overlapping row",()=>{
  const overlap=[rows[0],{id:"simultaneous",trackId:"native-system-id",startUs:500000,endUs:1500000},
    {id:"import",trackId:"import-id",startUs:0,endUs:2000000},
    {id:"unknown",trackId:"mic-looking-but-unregistered",startUs:0,endUs:2000000},
    {id:"missing",startUs:0,endUs:2000000}];
  const kinds=new Map([...tracks,["import-id","imported"]]);
  const full={...timeline,scope:"meeting" as const,analysisId:null,utteranceId:null};
  for(const [trackKind,expected] of [["mic",[0]],["system",[1]],["both",[0,1]],["imported",[2]],["processed",[]]] as const){
    assert.deepEqual(playingRows(overlap,{...full,trackKind},"m","analysis",kinds),expected);
    assert.equal(followingRow(overlap,{...full,trackKind},"m","analysis",kinds),expected[0]??null);
  }
  assert.deepEqual(playingRows(overlap,{...full,trackKind:"both"},"m","analysis",null),[]);
  for(const patch of [{playing:false},{meetingId:"other"},{timeUs:NaN},{timeUs:-1},{timeUs:Infinity}]){
    assert.deepEqual(playingRows(overlap,{...full,trackKind:"both",...patch},"m","analysis",kinds),[]);
  }
  assert.deepEqual(playingRows([{id:"bad",trackId:"native-mic-id",startUs:0,endUs:NaN}],full,"m","analysis",kinds),[]);
});
test("split utterance follows only its source interval and does not invent a split position",()=>{
  const split=[{id:"left",sourceUtteranceId:"u",startUs:0,endUs:1000000},{id:"right",sourceUtteranceId:"u",startUs:1000000,endUs:2000000}];
  assert.equal(followingRow(split,timeline,"m","analysis"),1);
  assert.equal(followingRow(split,{...timeline,timeUs:500000},"m","analysis"),0);
});
test("follow geometry scrolls only one port and keeps visible or tall rows stable",()=>{
  const view={top:100,bottom:400,scrollTop:200,maxScroll:1000};
  assert.equal(followingScrollTop(view,{top:120,bottom:350}),null);
  assert.equal(followingScrollTop(view,{top:500,bottom:550}),592);
  assert.equal(followingScrollTop(view,{top:-200,bottom:-100}),0);
  assert.equal(followingScrollTop(view,{top:2000,bottom:2500}),1000);
  assert.equal(followingScrollTop(view,{top:110,bottom:900}),null);
  assert.equal(followingScrollTop(view,{top:500,bottom:1500}),592);
  assert.equal(followingScrollTop({...view,bottom:105},{top:200,bottom:300}),null);
  assert.equal(followingScrollTop(view,{top:NaN,bottom:300}),null);
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
