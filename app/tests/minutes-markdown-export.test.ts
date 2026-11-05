import test from "node:test";
import assert from "node:assert/strict";
import { MarkdownExportController, type MarkdownExportRecord, type MarkdownExportRequest } from "../src/minutes-markdown-export.ts";
import type { MarkdownDocument } from "../src/minutes-markdown-state.ts";
const operation="00000000-0000-4000-8000-000000000002";
const document:MarkdownDocument={meetingId:"meeting",operationId:"00000000-0000-4000-8000-000000000001",markdown:"# 人間\r\n",lockVersion:1,contentHash:"a".repeat(64),updatedAt:0};
async function receipt(request:MarkdownExportRequest,text=document.markdown):Promise<MarkdownExportRecord>{
  const bytes=new TextEncoder().encode(text),hash=await crypto.subtle.digest("SHA-256",bytes);
  return {id:request.operationId,meetingId:request.meetingId,exportedContentHash:request.expected.contentHash,
    outputContentHash:Array.from(new Uint8Array(hash),b=>b.toString(16).padStart(2,"0")).join(""),outputSizeBytes:bytes.length,
    destinationPath:"/synthetic/example.md",state:"succeeded",errorMessage:null,createdAt:0,exportedAt:1};
}
test("export acknowledges exact byte receipt and native cancellation without keeping a pending request",async()=>{
  let calls=0;const c=new MarkdownExportController("meeting",async r=>{calls++;return receipt(r);},async()=>{throw Error("must not read");},()=>operation);
  assert.equal((await c.begin(document))?.state,"succeeded");assert.equal(calls,1);assert.deepEqual(c.snapshot(),{busy:false,needsCheck:false,retry:false,error:""});
  const cancelled=new MarkdownExportController("meeting",async()=>null,async()=>{throw Error("must not read");},()=>operation);
  assert.equal(await cancelled.begin(document),null);assert.equal(cancelled.snapshot().needsCheck,false);
});
test("concurrent clicks invoke once, pending state is entered before digest/send, returned request cannot be mutated",async()=>{
  let calls=0,release=()=>{},ready=()=>{};
  const entered=new Promise<void>(resolve=>ready=resolve);
  const c=new MarkdownExportController("meeting",async r=>{calls++;const result=await receipt(r);r.expected.contentHash="changed";await new Promise<void>(resolve=>{release=resolve;ready();});return result;},async()=>{throw Error("must not read");},()=>operation);
  const a=c.begin(document),b=c.begin(document);assert.equal(a,b);assert.equal(c.snapshot().busy,true);
  await entered;release();await a;assert.equal(calls,1);
});
test("response loss requires a read-only result check; active status never reopens native dialog",async()=>{
  let sends=0,reads=0;const c=new MarkdownExportController("meeting",async()=>{sends++;throw Error("lost");},async()=>{reads++;return{active:true,record:null};},()=>operation);
  await assert.rejects(c.begin(document));await assert.rejects(c.begin(document));await c.check();
  assert.equal(sends,1);assert.equal(reads,1);assert.equal(c.snapshot().needsCheck,true);
});
test("lost success recovers original receipt even after a newer document exists",async()=>{
  let saved:MarkdownExportRecord|null=null,sends=0;const c=new MarkdownExportController("meeting",async r=>{sends++;saved=await receipt(r);throw Error("lost");},async()=>({active:false,record:saved}),()=>operation);
  await assert.rejects(c.begin(document));assert.equal((await c.check())?.state,"succeeded");assert.equal(sends,1);assert.equal(c.snapshot().retry,false);
});
test("inactive unrecorded status permits only explicit identical retry for the same saved source",async()=>{
  const requests:MarkdownExportRequest[]=[];const c=new MarkdownExportController("meeting",async r=>{requests.push(structuredClone(r));if(requests.length===1)throw Error("transport");return receipt(r);},async()=>({active:false,record:null}),()=>operation);
  await assert.rejects(c.begin(document));await c.check();assert.equal(requests.length,1);assert.equal(c.snapshot().retry,true);
  await c.begin(document);assert.deepEqual(requests[0],requests[1]);assert.equal(requests[1].expected.lockVersion,1);
});
test("confirmed stopped unrecorded conflict can explicitly export a newer saved body with a fresh operation",async()=>{
  const requests:MarkdownExportRequest[]=[],newer={...document,markdown:"newer",contentHash:"b".repeat(64),lockVersion:2};
  const c=new MarkdownExportController("meeting",async r=>{requests.push(r);if(requests.length===1)throw Error("stale source");return receipt(r,newer.markdown);},async()=>({active:false,record:null}));
  await assert.rejects(c.begin(document));await assert.rejects(c.begin(newer));assert.equal(requests.length,1);
  await c.check();assert.equal(requests.length,1);await c.begin(newer);
  assert.notEqual(requests[0].operationId,requests[1].operationId);assert.equal(requests[1].expected.lockVersion,2);
});
test("foreign, malformed, wrong bytes and prematurely successful receipts stay unresolved",async()=>{
  const mutations=[(r:MarkdownExportRecord)=>r.meetingId="other",(r:MarkdownExportRecord)=>r.id=crypto.randomUUID(),
    (r:MarkdownExportRecord)=>r.outputSizeBytes++, (r:MarkdownExportRecord)=>r.outputContentHash="b".repeat(64),
    (r:MarkdownExportRecord)=>r.exportedAt=null];
  for(const mutate of mutations){const c=new MarkdownExportController("meeting",async request=>{const r=await receipt(request);mutate(r);return r;},async()=>({active:false,record:null}),()=>operation);
    await assert.rejects(c.begin(document));assert.equal(c.snapshot().needsCheck,true);
  }
});
test("failed or uncertain publication never emits success and never automatically retries",async()=>{
  for(const state of ["failed","interrupted"] as const){let sends=0;const c=new MarkdownExportController("meeting",async request=>{sends++;return{...await receipt(request),state,exportedAt:null,errorMessage:"I/O failure"};},async()=>{throw Error("must not read");},()=>operation);
    assert.equal((await c.begin(document))?.state,state);assert.equal(c.snapshot().error,"I/O failure");assert.equal(sends,1);
  }
});
test("status read failure and lingering writing receipt preserve frozen request without new send",async()=>{
  let reads=0,saved:MarkdownExportRecord|null=null;const c=new MarkdownExportController("meeting",async r=>{saved={...await receipt(r),state:"writing",exportedAt:null};return saved;},async()=>{if(!reads++)throw Error("status lost");return{active:false,record:saved};},()=>operation);
  await assert.rejects(c.begin(document));await assert.rejects(c.check());assert.equal(c.snapshot().needsCheck,true);
  await c.check();assert.equal(c.snapshot().needsCheck,true);
});
test("invalid document and operation identifier are rejected before export IPC",async()=>{
  let sends=0;const c=new MarkdownExportController("meeting",async()=>{sends++;return null;},async()=>({active:false,record:null}),()=>"invalid");
  await assert.rejects(c.begin(document));await assert.rejects(c.begin({...document,meetingId:"other"}));assert.equal(sends,0);
});
test("malformed absent-result status cannot enable a new export",async()=>{
  for(const record of [undefined,false,0,"absent"]){let sends=0;
    const c=new MarkdownExportController("meeting",async()=>{sends++;throw Error("lost");},async()=>({active:false,record:record as never}),()=>operation);
    await assert.rejects(c.begin(document));await assert.rejects(c.check());assert.equal(c.snapshot().needsCheck,true);
    await assert.rejects(c.begin(document));assert.equal(sends,1);
  }
});
