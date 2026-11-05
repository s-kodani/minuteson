import test from "node:test";
import assert from "node:assert/strict";
import {checkedInputStatus,RecordingInputSession,inputLabel,type InputNotice} from "../src/recording-input-state.ts";
import {InputLevelClock} from "../src/input-level-state.ts";

const owner="735027d4-a03e-43d3-830a-21e767dfaf88";
const connected={state:"connected" as const,sourceEpoch:1,interruptions:0,reason:null};
const report=()=>({schemaVersion:4,ownerId:owner,revision:1,reportAgeMs:0,microphone:connected,system:connected,notices:[] as InputNotice[],noticeLoss:{microphone:0,system:0}});
const interrupted=(input:"microphone"|"system"="microphone",target:"main"|"recorder"="main"):InputNotice=>{
  const status={state:"disconnected" as const,sourceEpoch:1,interruptions:1,reason:input==="microphone"?"device_disconnected":"stream_stopped"};
  return {id:`${owner}:${input}:1:interrupted`,input,target,kind:"interrupted",interruptions:1,recipientId:null,delivery:{sequence:1,generation:1,leaseAgeMs:null,rendered:false,status,receiveId:null}};
};
const issue=(input:"microphone"|"system"="microphone")=>({...report(),revision:2,[input]:interrupted(input).delivery.status,notices:[interrupted(input)]});

test("only current queued status is accepted with exact owner, fields and track state",()=>{
  assert.deepEqual(checkedInputStatus(report(),owner),report());assert.deepEqual(checkedInputStatus(issue(),owner),issue());
  for(const bad of [null,[],{}, {...report(),schemaVersion:3},{...report(),ownerId:"wrong"},{...report(),revision:0},{...report(),reportAgeMs:NaN},{...report(),extra:true},
    {...report(),noticeLoss:undefined},{...report(),microphone:{...connected,sourceEpoch:0}},{...report(),microphone:{...connected,interruptions:1}},
    {...report(),microphone:{...connected,reason:"stream_stopped"}},{...issue(),microphone:{...issue().microphone,reason:null}}])assert.throws(()=>checkedInputStatus(bad,owner));
});
test("historical queue notices require exact event identity, delivery and bounded loss",()=>{
  const n=interrupted();
  for(const bad of [{...n,id:"wrong"},{...n,target:"all"},{...n,delivery:undefined},{...n,delivery:{...n.delivery,sequence:2}},
    {...n,delivery:{...n.delivery,generation:0}},{...n,delivery:{...n.delivery,rendered:true}},
    {...n,delivery:{...n.delivery,status:{...n.delivery.status,reason:"stream_stopped"}}}])assert.throws(()=>checkedInputStatus({...issue(),notices:[bad]},owner));
  assert.throws(()=>checkedInputStatus({...report(),notices:[n]},owner));
  assert.throws(()=>checkedInputStatus({...issue(),noticeLoss:{microphone:3,system:0}},owner));
  assert.deepEqual(checkedInputStatus(issue("system"),owner),issue("system"));
});
test("heartbeat freshness does not prove audio samples or save confirmation",()=>{
  const s=new RecordingInputSession();s.reset(owner);s.accept(report(),100);
  assert.equal(s.read(1100).microphone.state,"connected");assert.equal(s.read(1101).microphone.state,"stale");
  s.reset(owner);s.accept(report(),0);s.accept({...report(),revision:2},800);assert.equal(s.read(1001).microphone.state,"connected");assert.equal(s.read(1801).microphone.state,"stale");
});
test("source generations and queue loss cannot reverse",()=>{
  const s=new RecordingInputSession();s.reset(owner);s.accept(report(),0);s.accept(issue(),1);
  assert.equal(s.read(1).microphone.state,"disconnected");
  assert.throws(()=>s.accept({...report(),revision:3,microphone:{...connected,interruptions:1}},2));
  const recovered={...report(),revision:3,microphone:{...connected,sourceEpoch:2,interruptions:1},notices:[]};s.accept(recovered,3);
  assert.equal(s.read(3).microphone.state,"connected");
  s.accept({...recovered,revision:4,noticeLoss:{microphone:1,system:0}},4);
  assert.throws(()=>s.accept({...recovered,revision:5},5));
});
test("stop, ownership replacement and reversing clocks never resurrect a claim",()=>{
  const s=new RecordingInputSession();s.reset(owner);s.accept(report(),100);assert.equal(s.read(99).microphone.state,"unknown");assert.throws(()=>s.accept(report(),101));
  s.reset(owner);s.accept({...report(),microphone:{...connected,state:"stopped"}},0);assert.throws(()=>s.accept({...report(),revision:2},1));
  s.reset(null);assert.equal(s.read(200).microphone.state,"unknown");assert.throws(()=>s.accept(report(),200));
  s.reset("735027d4-a03e-43d3-830a-21e767dfaf89");assert.throws(()=>s.accept(report(),200));
});
test("paired input status remains independent and explicit",()=>{
  const s=new RecordingInputSession();s.reset(owner);s.accept({...report(),system:null},0);
  assert.equal(s.read(0).system.state,"unknown");assert.throws(()=>s.accept({...report(),revision:2},1));
  s.reset(owner);s.accept(report(),0);s.accept(issue("system"),1);
  assert.equal(s.read(1).microphone.state,"connected");assert.equal(s.read(1).system.state,"disconnected");
  assert.match(inputLabel(s.read(1).system,"system"),/取得処理が停止/u);
  assert.match(inputLabel({state:"unknown"},"microphone"),/確認できません/u);
});
test("sleep-related clock changes make both inputs stale without an audio claim",()=>{
  const clock=new InputLevelClock(100,100000),s=new RecordingInputSession();s.reset(owner);s.accept(report(),clock.read(100,100000));
  assert.equal(s.read(clock.read(101,101100)).microphone.state,"stale");
});
