import test from "node:test";
import assert from "node:assert/strict";
import {RecordingInputReceipts} from "../src/recording-input-receipts.ts";
import type {InputNotice,InputKind} from "../src/recording-input-state.ts";
const owner="735027d4-a03e-43d3-830a-21e767dfaf88",recipient="735027d4-a03e-43d3-830a-21e767dfaf89",other="735027d4-a03e-43d3-830a-21e767dfaf90";
const notice=(input:InputKind="microphone",counter=1,kind:InputNotice["kind"]="interrupted",target:InputNotice["target"]="main",recipientId:string|null=null):InputNotice=>({id:`${owner}:${input}:${counter}:${kind}`,input,target,kind,interruptions:counter,recipientId});
const receipt=(n=notice(),recipientId=recipient)=>({schemaVersion:1,ownerId:owner,noticeId:n.id,target:n.target,recipientId});
const create=()=>new RecordingInputReceipts(owner,recipient,"main");
test("notification waits for a strict backend receipt, then appears only once in this lifetime",()=>{
  const r=create(),n=notice(),[a]=r.observe([n],0);
  assert.deepEqual(a,{sessionId:owner,noticeId:n.id,recipientId:recipient,target:"main",input:"microphone"});
  assert.deepEqual(r.read(0),{notices:[],reception:{microphone:"pending",system:"none"}});
  assert.deepEqual(r.observe([n],900),[]);assert.equal(r.complete(a,receipt(n),900),true);
  assert.deepEqual(r.read(900).notices,[{...n,recipientId:recipient}]);
  assert.deepEqual(r.observe([n],950),[]);r.dismiss("microphone");
  r.observe([{...n,recipientId:recipient}],1000);assert.deepEqual(r.read(1000).notices,[]);
  assert.equal(r.read(1000).reception.microphone,"received");
});
test("fresh publication reconciles a lost receipt response without waiting for that RPC",()=>{
  const r=create(),n=notice(),[a]=r.observe([n],0);
  assert.deepEqual(r.observe([n],1000),[]);assert.equal(r.read(1000).reception.microphone,"unconfirmed");
  r.observe([{...n,recipientId:recipient}],1100);assert.equal(r.read(1100).notices.length,1);
  assert.equal(r.complete(a,receipt(n),1200),false);r.fail(a);
  assert.equal(r.read(1200).reception.microphone,"received");
});
test("new WebView suppresses a notice already accepted by the old recipient",()=>{
  const n=notice(),r=new RecordingInputReceipts(owner,other,"main");
  assert.deepEqual(r.observe([{...n,recipientId:recipient}],0),[]);
  assert.deepEqual(r.read(0).notices,[]);assert.equal(r.read(0).reception.microphone,"received");
  const [a]=r.observe([notice("microphone",2)],1000);
  assert.equal(r.complete(a,receipt(notice("microphone",2),other),1001),true);
  assert.equal(r.read(1001).notices[0].interruptions,2);
});
test("first acceptance race is won by the backend, not by optimistic local rendering",()=>{
  const r=create(),n=notice(),[a]=r.observe([n],0);
  assert.equal(r.complete(a,receipt(n,other),1),false);
  assert.deepEqual(r.read(1).notices,[]);assert.equal(r.read(1).reception.microphone,"received");
});
test("failure is explicit and retry is throttled without overlap",()=>{
  const r=create(),n=notice(),[a]=r.observe([n],0);r.fail(a);
  assert.equal(r.read(1).reception.microphone,"unconfirmed");assert.deepEqual(r.observe([n],999),[]);
  const [b]=r.observe([n],1000);assert.notEqual(a,b);assert.deepEqual(r.observe([n],3000),[]);
  r.fail(a);assert.equal(r.complete(b,receipt(n),3001),true);
});
test("malformed receipt never turns a pending event into success and remains retryable",()=>{
  for(const value of [null,[],{...receipt(),extra:true},{...receipt(),ownerId:other},{...receipt(),noticeId:"other"},{...receipt(),target:"recorder"},{...receipt(),recipientId:null},{...receipt(),schemaVersion:2}]){
    const r=create(),[a]=r.observe([notice()],0);assert.throws(()=>r.complete(a,value,1));r.fail(a);
    assert.equal(r.read(1).reception.microphone,"unconfirmed");assert.deepEqual(r.read(1).notices,[]);
    assert.equal(r.observe([notice()],1000).length,1);
  }
});
test("an old request cannot accept a new episode or recreate an absent event",()=>{
  const r=create(),n=notice(),[a]=r.observe([n],0),next=notice("microphone",2),[b]=r.observe([next],1000);
  assert.equal(r.complete(a,receipt(n),1001),false);assert.equal(r.read(1001).notices.length,0);
  r.fail(a);assert.equal(r.complete(b,receipt(next),1002),true);
  r.observe([],1003);assert.deepEqual(r.read(1003).notices,[]);assert.equal(r.complete(b,receipt(next),1004),false);
});
test("source read failure or stale input clears rendering but retains accepted watermarks",()=>{
  const r=create(),n=notice(),[a]=r.observe([n],0);r.complete(a,receipt(n),1);
  r.observe([],2);assert.equal(r.read(2).reception.microphone,"none");
  r.observe([{...n,recipientId:recipient}],3);assert.deepEqual(r.read(3).notices,[]);
  assert.equal(r.read(3).reception.microphone,"received");
});
test("a transient missing report cannot overlap a still pending receipt for the same event",()=>{
  const r=create(),n=notice(),[a]=r.observe([n],0);r.observe([],100);
  assert.deepEqual(r.observe([n],1100),[]);
  assert.equal(r.read(1100).notices.length,0);assert.equal(r.complete(a,receipt(n),1101),true);
});
test("microphone, system, interruption and recovery all have independent receipts",()=>{
  const r=create(),mic=notice(),pc=notice("system"),requests=r.observe([mic,pc],0);
  assert.equal(requests.length,2);r.complete(requests[1],receipt(pc),1);r.complete(requests[0],receipt(mic),2);
  assert.equal(r.read(2).notices.length,2);r.dismiss("microphone");assert.equal(r.read(3).notices[0].input,"system");
  const recovered=notice("microphone",1,"recovered"),[a]=r.observe([recovered,{...pc,recipientId:recipient}],1000);
  r.complete(a,receipt(recovered),1001);assert.equal(r.read(1001).notices.find(n=>n.input==="microphone")?.kind,"recovered");
});
test("wrong window never requests receipt or renders the other window's event",()=>{
  const r=create();assert.deepEqual(r.observe([notice("microphone",1,"interrupted","recorder")],0),[]);
  assert.deepEqual(r.read(0),{notices:[],reception:{microphone:"none",system:"none"}});
});
test("reversing or invalid clock permanently invalidates this receipt lifetime",()=>{
  for(const at of [-1,NaN,Infinity,Number.MAX_SAFE_INTEGER+1,99]){
    const r=create(),n=notice(),[a]=r.observe([n],100);r.read(at);
    assert.equal(r.complete(a,receipt(n),200),false);assert.deepEqual(r.observe([n],201),[]);
    assert.deepEqual(r.read(201).notices,[]);
  }
});
test("recording and recipient identities must be valid UUID4",()=>{
  assert.throws(()=>new RecordingInputReceipts("wrong",recipient,"main"));
  assert.throws(()=>new RecordingInputReceipts(owner,"wrong","main"));
});
