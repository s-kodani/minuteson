import test from "node:test";
import assert from "node:assert/strict";
import {RecordingInputQueue,type QueueAttempt} from "../src/recording-input-queue.ts";
import {RecordingInputSession,checkedInputStatus,checkedInputNoticeReceipt,type InputNotice} from "../src/recording-input-state.ts";
const owner="735027d4-a03e-43d3-830a-21e767dfaf88",a="ed134978-12fc-4c5e-8e3f-372acbd44ad0",b="539640c5-f04c-4d91-96f2-36e37008fe1d",op="da93a8dc-c860-478d-ae51-9b982f126a4f";
const n=(sequence=1,generation=1,recipientId:string|null=null,rendered=false):InputNotice=>{
  const count=Math.floor(sequence/2)+sequence%2,kind=sequence%2?"interrupted":"recovered";
  return {id:`${owner}:microphone:${count}:${kind}`,input:"microphone",target:"main",kind,interruptions:count,recipientId,delivery:{sequence,generation,leaseAgeMs:recipientId?0:null,receiveId:recipientId?op:null,rendered,status:{state:kind==="recovered"?"connected":"disconnected",sourceEpoch:Math.floor(sequence/2)+1,interruptions:count,reason:kind==="recovered"?null:"device_disconnected"}}};
};
const report=(notice=n())=>({schemaVersion:4,ownerId:owner,revision:3,reportAgeMs:0,microphone:{state:"connected",sourceEpoch:2,interruptions:1,reason:null},system:{state:"connected",sourceEpoch:2,interruptions:1,reason:null},notices:[notice],noticeLoss:{microphone:0,system:0}});
const receipt=(attempt:QueueAttempt,rendered=attempt.action!=="receive",recipientId=a)=>({schemaVersion:2,ownerId:owner,noticeId:attempt.noticeId,target:attempt.target,recipientId,delivery:{...attempt.notice.delivery,leaseAgeMs:0,rendered,receiveId:attempt.receiveId}});
const accepted=(r:QueueAttempt,who=a)=>({...r.notice,recipientId:who,delivery:{...r.notice.delivery!,leaseAgeMs:0,receiveId:r.receiveId}});
const create=()=>new RecordingInputQueue(owner,a,"main");
test("historical interruption is not current binding and fresh metadata cannot refresh capture evidence",()=>{
  const r=report();assert.deepEqual(checkedInputStatus(r,owner),r);
  const s=new RecordingInputSession();s.reset(owner);s.accept(r,0);assert.equal(s.read(0).microphone.state,"connected");assert.equal(s.read(0).notices[0].kind,"interrupted");
  assert.deepEqual(s.loss(),{microphone:0,system:0});assert.deepEqual(s.read(1001).notices,[]);
});
test("queued projection requires closed delivery fields, actual history and bounded loss",()=>{
  const r=report(),base=n();for(const bad of [{...r,noticeLoss:{microphone:3,system:0}},{...r,noticeLoss:undefined},{...r,notices:[{...base,delivery:undefined}]},
    {...r,notices:[{...base,delivery:{...base.delivery,sequence:2}}]},{...r,notices:[{...base,delivery:{...base.delivery,generation:0}}]},
    {...r,notices:[{...base,delivery:{...base.delivery,rendered:true}}]},{...r,notices:[{...base,delivery:{...base.delivery,receiveId:a}}]},
    {...r,notices:[{...base,recipientId:a}]},{...r,notices:[{...base,delivery:{...base.delivery,status:{...base.delivery!.status,reason:"stream_stopped"}}}]}])assert.throws(()=>checkedInputStatus(bad,owner));
});
test("receipt is not render or close; exact visible frame and confirmed render enable close",()=>{
  const q=create(),first=n(),[receive]=q.observe([first],0);assert.deepEqual(q.read(0).notices,[]);q.complete(receive,receipt(receive),1);
  const own=accepted(receive);assert.equal(q.observe([own],2).length,0);assert.equal(q.read(2).notices.length,1);
  q.rendered(first.id,1,3);const [render]=q.observe([own],3);assert.equal(render.action,"render");q.complete(render,receipt(render,true),4);
  q.dismiss("microphone");const [close]=q.observe([own],4);assert.equal(close.action,"close");q.complete(close,receipt(close,true),5);assert.deepEqual(q.read(5).notices,[]);
});
test("lost receive or renewal reply reconciles only the exact operation nonce",()=>{
  const q=create(),[pending]=q.observe([n()],0),own=accepted(pending);q.observe([own],1);assert.equal(q.read(1).notices.length,1);assert.equal(q.complete(pending,receipt(pending),2),false);
  const [renew]=q.observe([own],1000);assert.equal(renew.action,"receive");assert.deepEqual(q.observe([own],2100),[]);
  q.observe([{...own,delivery:{...own.delivery,receiveId:renew.receiveId}}],2200);assert.equal(q.complete(renew,receipt(renew),2201),false);
});
test("new WebView cannot impersonate old claim but may accept an unshown new generation",()=>{
  const q=new RecordingInputQueue(owner,b,"main");assert.deepEqual(q.observe([n(1,1,a)],0),[]);assert.deepEqual(q.read(0).notices,[]);
  const [receive]=q.observe([n(1,2)],5000);q.complete(receive,receipt(receive,false,b),5001);q.observe([accepted(receive,b)],5002);assert.equal(q.read(5002).notices[0].delivery!.generation,2);
  q.rendered(receive.noticeId,1,5003);assert.deepEqual(q.observe([accepted(receive,b)],5003),[]);
});
test("transient read failure retains pending slot and cannot fabricate presentation",()=>{
  const q=create(),[receive]=q.observe([n()],0);q.observe([],100);assert.deepEqual(q.observe([n()],1200),[]);q.complete(receive,receipt(receive),1201);q.observe([],1202);assert.deepEqual(q.read(1202).notices,[]);
});
test("dismissal before render confirmation preserves recoverability instead of invented completion",()=>{
  const q=create(),[receive]=q.observe([n()],0);q.complete(receive,receipt(receive),1);q.observe([accepted(receive)],2);q.dismiss("microphone");assert.deepEqual(q.observe([accepted(receive)],3),[]);
  assert.equal(q.observe([n(1,2)],5001)[0].action,"receive");
});
test("old request, callback and failure cannot affect new head generation",()=>{
  const q=create(),[old]=q.observe([n()],0),[next]=q.observe([n(1,2)],1000);assert.equal(q.complete(old,receipt(old),1001),false);q.fail(old);q.rendered(old.noticeId,1,1002);
  assert.equal(q.complete(next,receipt(next),1003),true);assert.deepEqual(q.observe([accepted(next)],1004),[]);
});
test("same-generation ownership and rendered proof cannot reverse across absent projections",()=>{
  const s=new RecordingInputSession();s.reset(owner);s.accept(report(n(1,1,a,true)),0);s.accept({...report(n(1,1,a,true)),notices:[]},1);
  for(const x of [n(1,1,b,true),n(1,1,a,false),n(1,2,a,false)])assert.throws(()=>s.accept(report(x),2));
  const other=new RecordingInputSession();other.reset(owner);other.accept(report(n(1,1,a)),0);assert.equal(other.accept(report(n(1,2,b)),1),true);
  other.accept({...report(n(1,2,b)),noticeLoss:{microphone:1,system:0}},2);assert.throws(()=>other.accept(report(n(1,2,b)),3));
});
test("receipt validates event evidence, generation and nonce rather than receiver UUID alone",()=>{
  const q=create(),[r]=q.observe([n()],0),valid=receipt(r);assert.equal(checkedInputNoticeReceipt(valid,owner,r.noticeId,"main",r.notice).schemaVersion,2);
  for(const bad of [{...valid,delivery:{...valid.delivery,generation:2}},{...valid,delivery:{...valid.delivery,sequence:2}},
    {...valid,delivery:{...valid.delivery,status:{...valid.delivery.status,sourceEpoch:2}}},{...valid,extra:true},
    {...valid,delivery:{...valid.delivery,receiveId:null}},{...valid,delivery:{...valid.delivery,receiveId:op}}])assert.throws(()=>q.complete(r,bad,1));
});
test("invalid or reversing WebView clock permanently invalidates pending presentation",()=>{
  const q=create(),[r]=q.observe([n()],100);q.read(99);assert.equal(q.complete(r,receipt(r),101),false);assert.deepEqual(q.observe([n()],102),[]);
});
test("status key order is not evidence mutation and does not reject an otherwise exact receipt",()=>{
  const q=create(),[r]=q.observe([n()],0),value=receipt(r),s=value.delivery.status;
  value.delivery.status={reason:s.reason,interruptions:s.interruptions,sourceEpoch:s.sourceEpoch,state:s.state};
  assert.equal(q.complete(r,value,1),true);
  const session=new RecordingInputSession();session.reset(owner);session.accept(report(),0);
  const next=report();next.microphone={reason:null,interruptions:1,sourceEpoch:2,state:"connected"};
  assert.equal(session.accept(next,1),true);
});
test("hung requests become explicit unconfirmed state without duplicate attempts or invented render",()=>{
  const q=create(),[receive]=q.observe([n()],0);
  assert.equal(q.read(999).reception.microphone,"pending");assert.equal(q.read(1000).reception.microphone,"unconfirmed");
  assert.deepEqual(q.observe([n()],1100),[]);q.complete(receive,receipt(receive),1101);
  const own=accepted(receive);q.observe([own],1102);q.rendered(own.id,1,1103);
  const [render]=q.observe([own],1103);assert.equal(render.action,"render");assert.equal(q.read(2103).reception.microphone,"unconfirmed");
  assert.equal(q.read(2103).notices[0].delivery!.rendered,false);
});
test("authoritative readback clears lost receive/render errors and hanging render slots without another confirmation",()=>{
  const q=create(),[receive]=q.observe([n()],0);q.fail(receive);
  assert.equal(q.read(1).reception.microphone,"unconfirmed");const own=accepted(receive);q.observe([own],2);
  assert.equal(q.read(2).reception.microphone,"received");q.rendered(own.id,1,3);const [render]=q.observe([own],3);q.fail(render);
  assert.equal(q.read(4).reception.microphone,"unconfirmed");const confirmed={...own,delivery:{...own.delivery,rendered:true}};
  assert.deepEqual(q.observe([confirmed],5),[]);assert.equal(q.read(5).reception.microphone,"received");
  const other=create(),[r]=other.observe([n()],0);other.complete(r,receipt(r),1);const a=accepted(r);other.observe([a],2);other.rendered(a.id,1,3);const [hung]=other.observe([a],3);
  other.observe([{...a,delivery:{...a.delivery,rendered:true}}],4);assert.equal(other.complete(hung,receipt(hung,true),5),false);
});
