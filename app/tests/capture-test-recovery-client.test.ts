import test from "node:test";
import assert from "node:assert/strict";
import {CaptureRecoveryClient,checkedCaptureRecovery,type CaptureRecovery} from "../src/capture-test-recovery-client.ts";
const clear=():CaptureRecovery=>({schemaVersion:1,revision:2,phase:"clear",runId:null,errorMessage:null});
const attention=():CaptureRecovery=>({...clear(),phase:"attention",runId:"735027d4-a03e-43d3-830a-21e767dfaf88",errorMessage:"取得側の終了は未確認"});
const deferred=()=>{let resolve!:(value:unknown)=>void;const promise=new Promise<unknown>(r=>{resolve=r;});return {promise,resolve};};
test("recovery response strictly binds state, ID, error and safe revision",()=>{
  assert.deepEqual(checkedCaptureRecovery(clear()),clear());assert.deepEqual(checkedCaptureRecovery(attention()),attention());
  const reordered={errorMessage:null,runId:null,phase:"clear",revision:2,schemaVersion:1};assert.deepEqual(checkedCaptureRecovery(reordered),clear());
  for(const value of [null,{},[],{...clear(),extra:true},{...clear(),revision:0},{...clear(),revision:1.5},{...clear(),revision:Number.MAX_SAFE_INTEGER+1},
    {...clear(),phase:"unknown"},{...clear(),runId:attention().runId},{...clear(),errorMessage:"unexpected"},{...attention(),errorMessage:null},
    {...attention(),errorMessage:""},{...attention(),errorMessage:"bad\0error"},{...attention(),runId:"../audio"},{...attention(),runId:undefined},
    {...attention(),errorMessage:"x".repeat(16385)}])assert.throws(()=>checkedCaptureRecovery(value));
});
test("initial and failed reads block capture and never retry collection",async()=>{
  let retries=0;const client=new CaptureRecoveryClient({read:async()=>{throw Error("read failed");},retry:async()=>{retries++;return clear();}});
  assert.equal(client.blocked(),true);await assert.rejects(client.poll());await assert.rejects(client.poll());assert.equal(retries,0);assert.equal(client.blocked(),true);
});
test("repeated attention reads retain the same owner without any collection write",async()=>{
  let retries=0;const client=new CaptureRecoveryClient({read:async()=>attention(),retry:async()=>{retries++;return clear();}});
  await client.poll();await client.poll();assert.equal(retries,0);assert.equal(client.blocked(),true);assert.equal(client.getSnapshot().snapshot?.runId,attention().runId);
});
test("manual retry checks the exact revision and a successful result releases the gate",async()=>{
  let value=attention(),retries:number[]=[];const client=new CaptureRecoveryClient({read:async()=>value,retry:async revision=>{retries.push(revision);return value={...clear(),revision:3};}});
  await client.poll();await assert.rejects(client.retry(1));assert.deepEqual(retries,[]);await client.retry(2);assert.deepEqual(retries,[2]);assert.equal(client.blocked(),false);
});
test("changed confirmation and clear state cannot start another collection",async()=>{
  let value=attention(),retries=0;const client=new CaptureRecoveryClient({read:async()=>value,retry:async()=>{retries++;return clear();}});
  await client.poll();value={...attention(),revision:3};await assert.rejects(client.retry(2));value={...clear(),revision:4};await assert.rejects(client.retry(4));assert.equal(retries,0);
});
test("lost retry acknowledgement is reconciled by reads, never by resending",async()=>{
  let value=attention(),retries=0;const client=new CaptureRecoveryClient({read:async()=>value,retry:async()=>{retries++;value={...clear(),revision:3};throw Error("lost acknowledgement");}});
  await client.poll();await assert.rejects(client.retry(2));assert.equal(client.blocked(),true);await client.poll();assert.equal(client.blocked(),false);assert.equal(retries,1);
});
test("still-live helper returns new attention and requires a separate explicit retry",async()=>{
  let value=attention(),retries=0;const client=new CaptureRecoveryClient({read:async()=>value,retry:async()=>{retries++;return value={...attention(),revision:value.revision+1};}});
  await client.poll();await client.retry(2);await client.poll();assert.equal(client.blocked(),true);assert.equal(retries,1);await client.retry(3);assert.equal(retries,2);
});
test("simultaneous retries share the read and only one command can be accepted",async()=>{
  let hold=false,retries=0;const read=deferred(),reply=deferred();const client=new CaptureRecoveryClient({read:async()=>hold?read.promise:attention(),retry:async()=>{retries++;return reply.promise;}});
  await client.poll();hold=true;const first=client.retry(2),second=client.retry(2);read.resolve(attention());await assert.rejects(second);assert.equal(retries,1);assert.equal(client.blocked(),true);
  reply.resolve({...clear(),revision:3});await first;assert.equal(client.blocked(),false);
});
test("contradictory same revision and backward lifecycle are not trusted",async()=>{
  let value:unknown=attention();const client=new CaptureRecoveryClient({read:async()=>value,retry:async()=>clear()});await client.poll();value={...clear()};await assert.rejects(client.poll());
  value={...clear(),phase:"checking",revision:3};await assert.rejects(client.poll());assert.equal(client.blocked(),true);
});
test("read proof reconciles a collector whose acknowledgement never finishes, without replay",async()=>{
  let value=attention(),retries=0;const reply=deferred();const client=new CaptureRecoveryClient({read:async()=>value,retry:async()=>{retries++;return reply.promise;}});
  await client.poll();const operation=client.retry(2);await new Promise(resolve=>setImmediate(resolve));assert.equal(client.getSnapshot().commandBusy,true);
  value={...clear(),revision:3};await client.poll();assert.equal(client.blocked(),false);assert.equal(retries,1);
  reply.resolve(attention());await operation;assert.equal(client.getSnapshot().snapshot?.phase,"clear");assert.equal(client.blocked(),false);
});
