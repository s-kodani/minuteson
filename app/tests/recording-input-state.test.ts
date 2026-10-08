import test from "node:test";
import assert from "node:assert/strict";
import {checkedInputStatus,RecordingInputSession,microphoneLabel} from "../src/recording-input-state.ts";
import {InputLevelClock} from "../src/input-level-state.ts";
const id="735027d4-a03e-43d3-830a-21e767dfaf88";
const report=()=>({schemaVersion:1,ownerId:id,revision:1,reportAgeMs:0,microphone:{state:"connected",sourceEpoch:1,interruptions:0,reason:null},notice:null});
const issue=()=>({...report(),revision:2,microphone:{state:"disconnected",sourceEpoch:1,interruptions:1,reason:"device_disconnected"},notice:{id:`${id}:microphone:1:interrupted`,target:"main",kind:"interrupted",interruptions:1}});
test("binding status requires current owner, bounded counters, explicit reason and no extra fields",()=>{
  assert.deepEqual(checkedInputStatus(report(),id),report());assert.deepEqual(checkedInputStatus(issue(),id),issue());
  for(const r of [null,[],{}, {...report(),ownerId:"wrong"},{...report(),revision:0},{...report(),reportAgeMs:NaN},{...report(),extra:true},
    {...report(),microphone:{...report().microphone,sourceEpoch:0}},{...report(),microphone:{...report().microphone,sourceEpoch:Number.MAX_SAFE_INTEGER+1}},
    {...report(),microphone:{...report().microphone,interruptions:-1}},{...report(),microphone:{state:"connected",sourceEpoch:1,interruptions:0}},
    {...report(),microphone:{...report().microphone,state:"saved"}},{...report(),microphone:{...report().microphone,state:["connected"]}},{...report(),microphone:{...report().microphone,interruptions:1}},{...report(),microphone:{...report().microphone,reason:"unknown"}},
    {...issue(),microphone:{...issue().microphone,reason:null}},{...issue(),microphone:{...issue().microphone,interruptions:0}}])assert.throws(()=>checkedInputStatus(r,id));
});
test("notifications require the exact current owner, episode, state and one approved destination",()=>{
  for(const n of [{...issue().notice,id:"other"},{...issue().notice,target:"all"},{...issue().notice,target:["main"]},{...issue().notice,kind:["interrupted"]},{...issue().notice,interruptions:2},{...issue().notice,kind:"recovered"},{...issue().notice,extra:true}])assert.throws(()=>checkedInputStatus({...issue(),notice:n},id));
  assert.throws(()=>checkedInputStatus({...report(),notice:issue().notice},id));
});
test("connection heartbeats expire with RPC latency and never prove PCM reception or saving",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),100);
  assert.equal(s.read(1100).microphone.state,"connected");assert.equal(s.read(1101).microphone.state,"stale");assert.equal(s.read(1102).notice,null);
  assert.equal("frames" in s.read(1102).microphone,false);assert.equal("rms" in s.read(1102).microphone,false);
});
test("same publication rereads and touched metadata cannot reset the first local deadline",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),0);s.accept(report(),800);assert.equal(s.read(1001).microphone.state,"stale");
  s.accept({...report(),revision:2},1100);assert.equal(s.read(1100).microphone.state,"connected");
});
test("routing metadata may become available without changing an immutable publication or refreshing it",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept({...issue(),notice:null},0);s.accept(issue(),800);
  assert.equal(s.read(800).notice?.target,"main");assert.equal(s.read(1001).notice,null);
  assert.throws(()=>s.accept({...issue(),microphone:{...issue().microphone,reason:"engine_changed"}},1002));
});
test("old revisions cannot replace interruption and source counters cannot reverse",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),0);s.accept(issue(),1);assert.equal(s.accept(report(),2),false);assert.equal(s.read(2).microphone.state,"disconnected");
  assert.throws(()=>s.accept({...report(),revision:3},3));
  assert.throws(()=>s.accept({...issue(),revision:3,microphone:{...issue().microphone,sourceEpoch:0}},3));
});
test("confirmed new incarnation recovers after reconnecting without resetting recording ownership",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),0);s.accept(issue(),1);
  assert.throws(()=>s.accept({...report(),revision:3,microphone:{...report().microphone,interruptions:1}},2));
  s.accept({...issue(),revision:3,microphone:{...issue().microphone,state:"reconnecting",sourceEpoch:2}},2);
  s.accept({...report(),revision:4,microphone:{...report().microphone,sourceEpoch:2,interruptions:1},notice:{...issue().notice,id:`${id}:microphone:1:recovered`,kind:"recovered",target:"recorder"}},3);
  assert.equal(s.read(3).microphone.state,"connected");assert.equal(s.read(3).notice?.kind,"recovered");
});
test("stop, owner replacement, and reversing clocks cannot resurrect connection or notifications",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),100);assert.equal(s.read(99).microphone.state,"unknown");assert.throws(()=>s.accept(report(),101));
  s.reset(id);s.accept({...report(),microphone:{...report().microphone,state:"stopped"}},0);assert.throws(()=>s.accept({...report(),revision:2},1));
  s.reset(null);assert.equal(s.read(200).microphone.state,"unknown");assert.throws(()=>s.accept(report(),200));
  s.reset("735027d4-a03e-43d3-830a-21e767dfaf89");assert.throws(()=>s.accept(report(),200));
});
test("paused WebView clock during sleep cannot retain a connected or recovered claim",()=>{
  const c=new InputLevelClock(100,100000),s=new RecordingInputSession();s.reset(id);s.accept(report(),c.read(100,100000));
  assert.equal(s.read(c.read(101,101100)).microphone.state,"stale");
});
test("format changes and unknown reports remain explicit and do not suggest another default input",()=>{
  assert.match(microphoneLabel({...checkedInputStatus(issue(),id).microphone,state:"unavailable",reason:"format_changed"}),/入力形式が変更/u);
  assert.match(microphoneLabel({state:"unknown"}),/確認できません/u);
  assert.doesNotMatch(microphoneLabel(checkedInputStatus(issue(),id).microphone),/保存中|PCの音声は録音中|自動切替/u);
});
