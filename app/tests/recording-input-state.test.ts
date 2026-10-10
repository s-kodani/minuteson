import test from "node:test";
import assert from "node:assert/strict";
import {checkedInputStatus,checkedInputNoticeReceipt,RecordingInputSession,microphoneLabel,systemLabel} from "../src/recording-input-state.ts";
import {InputLevelClock} from "../src/input-level-state.ts";
const id="735027d4-a03e-43d3-830a-21e767dfaf88";
const report=()=>({schemaVersion:3,ownerId:id,revision:1,reportAgeMs:0,microphone:{state:"connected",sourceEpoch:1,interruptions:0,reason:null},system:{state:"connected",sourceEpoch:1,interruptions:0,reason:null},notices:[]});
const issue=()=>({...report(),revision:2,microphone:{state:"disconnected",sourceEpoch:1,interruptions:1,reason:"device_disconnected"},notices:[{id:`${id}:microphone:1:interrupted`,input:"microphone",target:"main",kind:"interrupted",interruptions:1,recipientId:null as string|null}]});
test("binding status requires current owner, bounded counters, explicit reason and no extra fields",()=>{
  assert.deepEqual(checkedInputStatus(report(),id),report());assert.deepEqual(checkedInputStatus(issue(),id),issue());
  for(const r of [null,[],{}, {...report(),ownerId:"wrong"},{...report(),revision:0},{...report(),reportAgeMs:NaN},{...report(),extra:true},
    {...report(),microphone:{...report().microphone,sourceEpoch:0}},{...report(),microphone:{...report().microphone,sourceEpoch:Number.MAX_SAFE_INTEGER+1}},
    {...report(),microphone:{...report().microphone,interruptions:-1}},{...report(),microphone:{state:"connected",sourceEpoch:1,interruptions:0}},
    {...report(),microphone:{...report().microphone,state:"saved"}},{...report(),microphone:{...report().microphone,state:["connected"]}},{...report(),microphone:{...report().microphone,interruptions:1}},{...report(),microphone:{...report().microphone,reason:"unknown"}},
    {...issue(),microphone:{...issue().microphone,reason:null}},{...issue(),microphone:{...issue().microphone,interruptions:0}}])assert.throws(()=>checkedInputStatus(r,id));
});
test("notifications require the exact current owner, episode, state and one approved destination",()=>{
  for(const n of [{...issue().notices[0],id:"other"},{...issue().notices[0],target:"all"},{...issue().notices[0],target:["main"]},{...issue().notices[0],kind:["interrupted"]},{...issue().notices[0],interruptions:2},{...issue().notices[0],kind:"recovered"},{...issue().notices[0],extra:true}])assert.throws(()=>checkedInputStatus({...issue(),notices:[n]},id));
  assert.throws(()=>checkedInputStatus({...report(),notices:issue().notices},id));
});
test("connection heartbeats expire with RPC latency and never prove PCM reception or saving",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),100);
  assert.equal(s.read(1100).microphone.state,"connected");assert.equal(s.read(1101).microphone.state,"stale");assert.deepEqual(s.read(1102).notices,[]);
  assert.equal("frames" in s.read(1102).microphone,false);assert.equal("rms" in s.read(1102).microphone,false);
});
test("same publication rereads and touched metadata cannot reset the first local deadline",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),0);s.accept(report(),800);assert.equal(s.read(1001).microphone.state,"stale");
  s.accept({...report(),revision:2},1100);assert.equal(s.read(1100).microphone.state,"connected");
});
test("routing metadata may become available without changing an immutable publication or refreshing it",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept({...issue(),notices:[]},0);s.accept(issue(),800);
  assert.equal(s.read(800).notices[0]?.target,"main");assert.deepEqual(s.read(1001).notices,[]);
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
  s.accept({...report(),revision:4,microphone:{...report().microphone,sourceEpoch:2,interruptions:1},notices:[{...issue().notices[0],id:`${id}:microphone:1:recovered`,kind:"recovered",target:"recorder"}]},3);
  assert.equal(s.read(3).microphone.state,"connected");assert.equal(s.read(3).notices[0]?.kind,"recovered");
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
const systemIssue=()=>({...report(),revision:2,system:{state:"disconnected",sourceEpoch:1,interruptions:1,reason:"stream_stopped"},notices:[{id:`${id}:system:1:interrupted`,input:"system",target:"recorder",kind:"interrupted",interruptions:1,recipientId:null}]});
test("clock recovery remains disconnected on both inputs until confirmed new source epochs",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),0);
  const paused={...report(),revision:2,microphone:{...issue().microphone,reason:"clock_changed"},system:{...systemIssue().system,reason:"clock_changed"},notices:[]};
  s.accept(paused,1);
  const visible=s.read(1);
  assert.match(microphoneLabel(visible.microphone),/切断されました.*スリープ等により取得時計が変更/u);
  assert.match(systemLabel(visible.system),/切断されました.*スリープ等により取得時計が変更/u);
  assert.doesNotMatch(microphoneLabel(visible.microphone),/保存済み|正常|自動切替/u);
  assert.throws(()=>s.accept({...report(),revision:3,microphone:{...report().microphone,interruptions:1},system:{...report().system,interruptions:1}},2));
  const recovered={...report(),revision:3,microphone:{...report().microphone,sourceEpoch:2,interruptions:1},system:{...report().system,sourceEpoch:2,interruptions:1}};
  s.accept(recovered,3);assert.equal(s.read(3).microphone.state,"connected");assert.equal(s.read(3).system.state,"connected");
});
test("schema three requires both tracks explicitly and never guesses an unsupported system binding",()=>{
  const missing=report() as Record<string,unknown>;delete missing.system;
  for(const r of [missing,{...report(),schemaVersion:1},{...report(),schemaVersion:2},{...report(),system:undefined},{...report(),system:[]},{...report(),system:{...report().system,reason:undefined}},
    {...report(),system:{...report().system,sourceEpoch:0}},{...report(),system:{...report().system,sourceEpoch:Number.MAX_SAFE_INTEGER+1}},
    {...report(),system:{...report().system,interruptions:1}},{...report(),system:{...report().system,state:"saved"}},{...systemIssue(),system:{...systemIssue().system,reason:"device_disconnected"}},
    {...issue(),microphone:{...issue().microphone,reason:"stream_stopped"}}])assert.throws(()=>checkedInputStatus(r,id));
  const s=new RecordingInputSession();s.reset(id);s.accept({...report(),system:null},0);
  assert.equal(s.read(0).microphone.state,"connected");assert.equal(s.read(0).system.state,"unknown");assert.deepEqual(s.read(0).notices,[]);
  assert.throws(()=>s.accept({...report(),revision:2},1));
  s.reset(id);s.accept(report(),0);assert.throws(()=>s.accept({...report(),revision:2,system:null},1));
});
test("notice receipt projection is strict and the accepted recipient cannot be reset or replaced",()=>{
  const recipient="735027d4-a03e-43d3-830a-21e767dfaf89",other="735027d4-a03e-43d3-830a-21e767dfaf90";
  const s=new RecordingInputSession();s.reset(id);s.accept(issue(),0);
  const accepted={...issue(),notices:[{...issue().notices[0],recipientId:recipient}]};s.accept(accepted,800);
  assert.equal(s.read(800).notices[0].recipientId,recipient);
  assert.throws(()=>s.accept(issue(),801));assert.throws(()=>s.accept({...accepted,revision:3,notices:[{...accepted.notices[0],recipientId:other}]},802));
  assert.deepEqual(s.read(1001).notices,[]); // Receipt metadata cannot refresh capture evidence.
  for(const recipientId of [undefined,"",{},recipient.toUpperCase()])assert.throws(()=>checkedInputStatus({...issue(),notices:[{...issue().notices[0],recipientId}]},id));
  const n=issue().notices[0],receipt={schemaVersion:1,ownerId:id,noticeId:n.id,target:"main" as const,recipientId:recipient};
  assert.deepEqual(checkedInputNoticeReceipt(receipt,id,n.id,"main"),receipt);
  for(const value of [null,[],{...receipt,extra:true},{...receipt,schemaVersion:3},{...receipt,ownerId:other},{...receipt,noticeId:"other"},{...receipt,target:"recorder"},{...receipt,recipientId:null}])assert.throws(()=>checkedInputNoticeReceipt(value,id,n.id,"main"));
});
test("system-specific reasons and each notification must match their own input",()=>{
  assert.deepEqual(checkedInputStatus(systemIssue(),id),systemIssue());
  for(const reason of ["stream_stopped","application_unavailable","acquisition_timeout","permission_unavailable","restart_failed","format_changed"])
    assert.doesNotThrow(()=>checkedInputStatus({...systemIssue(),system:{...systemIssue().system,reason}},id));
  const n=systemIssue().notices[0];
  for(const r of [{...systemIssue(),system:null},{...systemIssue(),notices:[n,n]},
    {...systemIssue(),notices:[n,n,n]},{...systemIssue(),notices:[{...n,input:"microphone"}]},
    {...systemIssue(),notices:[{...n,id:`${id}:microphone:1:interrupted`}]},{...systemIssue(),notices:[{...n,input:"pc"}]}])assert.throws(()=>checkedInputStatus(r,id));
  assert.match(systemLabel(checkedInputStatus({...systemIssue(),system:{...systemIssue().system,reason:"permission_unavailable"}},id).system!),/PC音声の権限/u);
  assert.match(systemLabel(checkedInputStatus(systemIssue(),id).system!),/取得処理が停止/u);
  assert.doesNotMatch(systemLabel(checkedInputStatus(systemIssue(),id).system!),/保存中|自動切替/u);
});
test("system interruption and verified recovery leave the microphone binding independent",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),0);s.accept(systemIssue(),1);
  assert.equal(s.read(1).microphone.state,"connected");assert.equal(s.read(1).system.state,"disconnected");
  assert.equal(s.accept(report(),2),false);
  assert.throws(()=>s.accept({...report(),revision:3},3));
  s.accept({...systemIssue(),revision:3,system:{...systemIssue().system,state:"reconnecting",sourceEpoch:2}},4);
  s.accept({...report(),revision:4,system:{...report().system,sourceEpoch:2,interruptions:1},notices:[{...systemIssue().notices[0],id:`${id}:system:1:recovered`,kind:"recovered"}]},5);
  assert.equal(s.read(5).microphone.state,"connected");assert.equal(s.read(5).system.state,"connected");assert.equal(s.read(5).notices[0]?.input,"system");
  assert.throws(()=>s.accept({...report(),revision:5,system:{...report().system,sourceEpoch:2}},6));
  assert.throws(()=>s.accept({...report(),revision:5,system:{...report().system,interruptions:1}},6));
});
test("simultaneous input notices are independent even at the same interruption number",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept(report(),0);
  const both={...issue(),system:systemIssue().system,notices:[...issue().notices,...systemIssue().notices]};s.accept(both,1);
  const v=s.read(1);assert.equal(v.microphone.state,"disconnected");assert.equal(v.system.state,"disconnected");
  assert.equal(v.notices.length,2);assert.notEqual(v.notices[0].id,v.notices[1].id);assert.deepEqual(v.notices.map(n=>n.target),["main","recorder"]);
  assert.deepEqual(s.read(1002),{microphone:{state:"stale"},system:{state:"stale"},notices:[]});
});
test("the same system publication cannot mutate or reset either freshness deadline",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept({...systemIssue(),notices:[]},0);s.accept(systemIssue(),800);
  assert.equal(s.read(800).notices[0]?.input,"system");assert.throws(()=>s.accept({...systemIssue(),system:{...systemIssue().system,reason:"restart_failed"}},801));
  assert.equal(s.read(1001).microphone.state,"stale");assert.equal(s.read(1001).system.state,"stale");
});
test("system stop is sticky even if microphone heartbeats and revisions continue",()=>{
  const s=new RecordingInputSession();s.reset(id);s.accept({...report(),system:{...report().system,state:"stopped"}},0);
  s.accept({...report(),revision:2,system:{...report().system,state:"stopped"}},1);
  assert.equal(s.read(1).microphone.state,"connected");assert.equal(s.read(1).system.state,"stopped");
  assert.throws(()=>s.accept({...report(),revision:3},2));
  assert.throws(()=>checkedInputStatus({...systemIssue(),system:{...systemIssue().system,state:"stopped"}},id));
  s.reset(null);assert.deepEqual(s.read(3),{microphone:{state:"unknown"},system:{state:"unknown"},notices:[]});
});
