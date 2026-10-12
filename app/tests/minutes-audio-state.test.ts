import test from "node:test";
import assert from "node:assert/strict";
import { markdownIntervalRequest, validateAudioSummary, validateMarkdownPlayback } from "../src/minutes-audio-state.ts";
import { ChunkPlayer, type PlaybackInfo, type PlaybackClock } from "../src/playback.ts";

const summary={schemaVersion:1 as const,durationUs:1000000,modes:["mic" as const]};
const request=markdownIntervalRequest("meeting",summary,{startUs:250001,endUs:700003});
const info:PlaybackInfo={token:"interval",sampleRate:48000,channels:1,totalFrames:21600,startUs:250001,endUs:700003,trackKind:"mic",alignment:"unverified",
  missingRanges:[{trackKind:"mic",startUs:250001,endUs:300000,startFrame:0,endFrame:2399,reasonCode:"capture_timestamp_gap"}]};

test("Markdown interval chooses available original audio without a transcript or model",()=>{
  assert.equal(request.mode,"mic");
  assert.equal(markdownIntervalRequest("m",{...summary,modes:["mic","system","both"]},request).mode,"both");
  assert.equal(markdownIntervalRequest("m",{...summary,modes:["imported"]},request).mode,"imported");
  assert.equal(markdownIntervalRequest("m",{...summary,modes:["system"]},request).mode,"system");
  assert.throws(()=>markdownIntervalRequest("m",{...summary,modes:[]},request));
});
test("summary refuses duplicate, foreign, contradictory modes and unsafe duration",()=>{
  for(const invalid of [{...summary,durationUs:NaN},{...summary,durationUs:0},{...summary,modes:["mic","mic"]},{...summary,modes:["both"]},{...summary,modes:["processed"]},{...summary,modes:["imported","mic"]}])
    assert.throws(()=>validateAudioSummary(invalid as never));
});
test("interval validation preserves fractional microseconds and uses relative gaps",()=>{
  assert.doesNotThrow(()=>validateMarkdownPlayback(info,request));
  for(const invalid of [{...info,startUs:250000},{...info,endUs:700004},{...info,totalFrames:21599},{...info,trackKind:"system"},
    {...info,missingRanges:[{...info.missingRanges![0],startUs:200000}]},
    {...info,missingRanges:[{...info.missingRanges![0],endFrame:2400}]}])assert.throws(()=>validateMarkdownPlayback(invalid,request));
});
test("interval admission refuses out of range, reversed and unsafe times",()=>{
  for(const range of [{startUs:-1,endUs:1},{startUs:1,endUs:1},{startUs:0,endUs:1000001},{startUs:0.1,endUs:1},{startUs:0,endUs:NaN}])
    assert.throws(()=>markdownIntervalRequest("m",summary,range));
});
test("interval gap stops playback instead of reading or manufacturing silent samples",async()=>{
  let reads=0;
  const clock:PlaybackClock={now:()=>0,resume:async()=>{},schedule:()=>()=>{}};
  const player=new ChunkPlayer(clock,async()=>{reads++;throw Error("must not read gap");},()=>{});
  player.load(info);await player.play();assert.equal(reads,0);assert.equal(player.snapshot().playing,false);
  player.seek(2399);await player.play();assert.equal(reads,1);
});
test("clipped tail gaps retain absolute microseconds and stop at the relative interval end",()=>{
  const r=markdownIntervalRequest("m",summary,{startUs:123456,endUs:300001});
  const clipped:PlaybackInfo={...info,startUs:123456,endUs:300001,totalFrames:8475,
    missingRanges:[{trackKind:"mic",startUs:200003,endUs:300001,startFrame:3674,endFrame:8475,reasonCode:"capture_stop_gap"}]};
  assert.doesNotThrow(()=>validateMarkdownPlayback(clipped,r));
  assert.throws(()=>validateMarkdownPlayback({...clipped,missingRanges:[{...clipped.missingRanges![0],endFrame:8476}]},r));
});
test("interval response preserves both track gaps but refuses unrelated sources and malformed reasons",()=>{
  const r=markdownIntervalRequest("m",{...summary,modes:["mic","system","both"]},request);
  const mixed:PlaybackInfo={...info,trackKind:"both",missingRanges:[...info.missingRanges!,{...info.missingRanges![0],trackKind:"system"}]};
  assert.doesNotThrow(()=>validateMarkdownPlayback(mixed,r));
  assert.throws(()=>validateMarkdownPlayback({...mixed,missingRanges:[{...info.missingRanges![0],trackKind:"imported"}]},r));
  assert.throws(()=>validateMarkdownPlayback({...info,missingRanges:[{...info.missingRanges![0],reasonCode:"bad\nreason"}]},request));
});
