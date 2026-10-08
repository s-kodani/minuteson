import test from "node:test";
import assert from "node:assert/strict";
import {checkedInputLevels,InputLevelClock,InputLevelSession,visibleLevel,type LevelTrack} from "../src/input-level-state.ts";
const id="735027d4-a03e-43d3-830a-21e767dfaf88",other="735027d4-a03e-43d3-830a-21e767dfaf89";
const measured:LevelTrack={state:"measured",windowFrames:1024,ageMs:20,rms:0.5,peak:1};
const waiting:LevelTrack={state:"waiting",windowFrames:null,ageMs:null,rms:null,peak:null};
const report=()=>({schemaVersion:1,ownerId:id,revision:1,reportAgeMs:30,microphone:{...measured},system:{...waiting}});
test("bounded telemetry keeps both independent tracks without inferring connection or silence",()=>{
  const r=checkedInputLevels(report(),id);assert.deepEqual(r.microphone,measured);assert.deepEqual(r.system,waiting);
});
test("zero is measured silence with no fabricated finite decibel value",()=>{
  assert.deepEqual(visibleLevel({...measured,rms:0,peak:0},0,0),{state:"measured",rms:0,peak:0,rmsDbFS:null,peakDbFS:null,clipping:false});
  assert.deepEqual(visibleLevel(waiting,0,0),{state:"waiting"});
});
test("actual amplitudes determine dBFS and clipping; visualization need not clamp the evidence",()=>{
  const level=visibleLevel({...measured,rms:0.5,peak:2},0,0);assert.equal(level.state,"measured");
  if(level.state==="measured"){assert.equal(level.clipping,true);assert.equal(level.peak,2);assert.ok(Math.abs(level.rmsDbFS!+6.020599913)<1e-8);assert.ok(Math.abs(level.peakDbFS!-6.020599913)<1e-8);}
});
test("wrong owner, invalid version/age, missing and unknown fields are rejected",()=>{
  const base=report();
  for(const bad of [null,[],{...base,ownerId:other},{...base,ownerId:id.toUpperCase()},{...base,schemaVersion:2},{...base,revision:0},{...base,revision:1.5},{...base,revision:Number.MAX_SAFE_INTEGER+1},{...base,reportAgeMs:-1},{...base,extra:0},{...base,system:undefined}])assert.throws(()=>checkedInputLevels(bad,id));
});
test("non-finite, impossible or incomplete PCM measurements never become meters",()=>{
  for(const microphone of [{...measured,rms:NaN},{...measured,peak:Infinity},{...measured,rms:-1},{...measured,rms:2},{...measured,peak:17},{...measured,windowFrames:0},{...measured,windowFrames:1_000_001},{...measured,ageMs:0.5},{...measured,peak:null},{...waiting,peak:0},{...measured,extra:0}])assert.throws(()=>checkedInputLevels({...report(),microphone},id));
});
test("callback age, native publication age and local elapsed time all count toward freshness",()=>{
  assert.equal(visibleLevel(measured,30,950).state,"measured");assert.equal(visibleLevel(measured,30,951).state,"stale");
  for(const bad of [-1,NaN,Infinity])assert.equal(visibleLevel(measured,0,bad).state,"unavailable");
});
test("repeated polling cannot freshen an old publication; lower revisions do not replace it",()=>{
  const s=new InputLevelSession();s.reset(id);assert.equal(s.accept(report(),100),true);
  assert.equal(s.accept({...report(),reportAgeMs:50},900),false);assert.equal(s.read(1100).microphone.state,"stale");
  assert.equal(s.accept({...report(),revision:2},1200),true);assert.equal(s.accept(report(),1250),false);assert.equal(s.read(1250).microphone.state,"measured");
});
test("same-revision contradictions and backward publication age are rejected",()=>{
  const s=new InputLevelSession();s.reset(id);s.accept(report(),100);
  assert.throws(()=>s.accept({...report(),microphone:{...measured,peak:2}},200));assert.throws(()=>s.accept({...report(),reportAgeMs:0},200));
});
test("owner reset, cleared state and local clock reversal do not reuse old values",()=>{
  const s=new InputLevelSession();assert.throws(()=>s.accept(report(),0));s.reset(id);s.accept(report(),100);
  assert.equal(s.read(90).microphone.state,"unavailable");assert.throws(()=>s.accept(report(),90));s.reset(other);
  assert.equal(s.read(200).microphone.state,"unavailable");assert.throws(()=>s.accept(report(),200));s.reset(null);assert.equal(s.read(200).system.state,"unavailable");
});
test("native age leaps forward are honored without resetting the local freshness baseline",()=>{
  const s=new InputLevelSession();s.reset(id);s.accept(report(),100);
  assert.equal(s.accept({...report(),reportAgeMs:90000},101),false);
  assert.equal(s.read(101).microphone.state,"stale");
  assert.throws(()=>s.accept({...report(),reportAgeMs:89999},102));
});
test("reversal after a later read invalidates the epoch until explicit reset",()=>{
  const s=new InputLevelSession();s.reset(id);s.accept(report(),100);s.read(900);
  assert.equal(s.read(800).microphone.state,"unavailable");assert.equal(s.read(901).microphone.state,"unavailable");
  assert.throws(()=>s.accept({...report(),revision:2},902));s.reset(id);s.accept({...report(),revision:2},902);assert.equal(s.read(903).microphone.state,"measured");
});
test("sleep advances freshness even if the WebView performance clock pauses",()=>{
  const clock=new InputLevelClock(100,100000),s=new InputLevelSession();s.reset(id);
  s.accept(report(),clock.read(100,100000));
  assert.equal(s.read(clock.read(101,190000)).microphone.state,"stale");
  s.accept({...report(),revision:2},clock.read(102,190001));
  assert.equal(s.read(clock.read(103,190002)).microphone.state,"measured");
});
test("wall or performance reversal invalidates the owner's clock instead of freshening old audio",()=>{
  for(const [mono,wall] of [[99,100001],[101,99999],[NaN,100001],[101,Infinity]]){
    const clock=new InputLevelClock(100,100000);
    assert.ok(Number.isNaN(clock.read(mono,wall)));
    assert.ok(Number.isNaN(clock.read(102,100002)));
  }
  assert.ok(Number.isNaN(new InputLevelClock(-1,100).read(0,101)));
});
