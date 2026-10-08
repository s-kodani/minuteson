import test from "node:test";
import assert from "node:assert/strict";
import {checkedWrites,WriteSession} from "../src/recording-write-state.ts";
import {InputLevelClock} from "../src/input-level-state.ts";
const id="735027d4-a03e-43d3-830a-21e767dfaf88";
const report=()=>({schemaVersion:1,ownerId:id,revision:1,microphone:{state:"written",frames:480,ageMs:10},system:{state:"waiting",frames:null,ageMs:null}});
test("strict write response binds both tracks, owner, safe counter and explicit nulls",()=>{
  assert.deepEqual(checkedWrites(report(),id),report());
  for(const value of [null,[],{}, {...report(),ownerId:"wrong"},{...report(),revision:0},{...report(),revision:1.5},{...report(),revision:Number.MAX_SAFE_INTEGER+1},{...report(),extra:true},
    {...report(),microphone:{state:"written",frames:0,ageMs:0}},{...report(),microphone:{state:"written",frames:1,ageMs:NaN}},{...report(),system:{state:"waiting",frames:0,ageMs:null}},
    {...report(),system:{state:"unavailable",frames:null}},{...report(),system:{state:"unavailable",frames:null,ageMs:null,extra:true}}])assert.throws(()=>checkedWrites(value,id));
});
test("fresh observed PCM is distinct from missing input and expires at one second",()=>{
  const s=new WriteSession();s.reset(id);s.accept(report(),100);assert.equal(s.read(1090).microphone.state,"written");assert.equal(s.read(1091).microphone.state,"stale");assert.equal(s.read(1091).system.state,"waiting");
});
test("new RPC revisions or touched timestamps cannot freshen unchanged PCM counts",()=>{
  const s=new WriteSession();s.reset(id);s.accept(report(),100);s.accept({...report(),revision:2,microphone:{state:"written",frames:480,ageMs:0}},900);
  assert.equal(s.read(1100).microphone.state,"stale");s.accept({...report(),revision:3,microphone:{state:"written",frames:960,ageMs:1}},1200);assert.equal(s.read(1201).microphone.state,"written");
});
test("old replies and same-revision contradictions cannot replace current write evidence",()=>{
  const s=new WriteSession();s.reset(id);s.accept(report(),100);assert.equal(s.accept(report(),900),false);assert.equal(s.read(1100).microphone.state,"stale");
  assert.throws(()=>s.accept({...report(),microphone:{state:"written",frames:481,ageMs:0}},1101));s.accept({...report(),revision:2},1102);assert.equal(s.accept(report(),1103),false);
});
test("temporary unknown state cannot erase the floor or allow frame counter reversal",()=>{
  const s=new WriteSession();s.reset(id);s.accept(report(),100);s.accept({...report(),revision:2,microphone:{state:"unavailable",frames:null,ageMs:null}},500);
  assert.equal(s.read(500).microphone.state,"unavailable");assert.throws(()=>s.accept({...report(),revision:3,microphone:{state:"written",frames:479,ageMs:0}},600));
  s.accept({...report(),revision:3},1100);assert.equal(s.read(1101).microphone.state,"stale");
});
test("owner reset and reversing clocks never reuse previous saved PCM",()=>{
  const s=new WriteSession();s.reset(id);s.accept(report(),100);assert.equal(s.read(99).microphone.state,"unavailable");assert.throws(()=>s.accept({...report(),revision:2},101));s.reset(null);assert.equal(s.read(200).system.state,"unavailable");assert.throws(()=>s.accept(report(),200));
});
test("RPC latency and system sleep remain part of write freshness",()=>{
  const s=new WriteSession(),c=new InputLevelClock(100,100000);s.reset(id);s.accept(report(),c.read(100,100000));
  assert.equal(s.read(c.read(101,101100)).microphone.state,"stale");
});
