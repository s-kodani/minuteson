import test from "node:test";
import assert from "node:assert/strict";
import { runInNewContext } from "node:vm";
import { DraftSaveQueue } from "../src/draft-save.ts";
import { createMinutesSaveQueue } from "../src/minutes-save.ts";
import { editInline, textNode } from "../src/minutes-edit.ts";
import type { MinutesContent, MinutesDraft } from "../src/minutes-state.ts";
import { installMinutesFixture } from "./minutes-browser-fixture.js";

async function fixture() {
  let initialize: (() => void) | undefined;
  await installMinutesFixture({addInitScript:async(callback:()=>void)=>{initialize=callback;},reload:async()=>{}});
  const fakeWindow:any={};
  runInNewContext(`(${initialize!.toString()})()`,{window:fakeWindow,structuredClone});
  return {data:fakeWindow.__mon,invoke:fakeWindow.__TAURI_INTERNALS__.invoke};
}

test("rich minutes save failure keeps acknowledged document, all anchors and latest local edit until explicit retry",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  const generated=JSON.stringify(data.views.a.content);
  const queue=new DraftSaveQueue<MinutesContent,MinutesDraft>((draftId,expectedLockVersion,payload)=>invoke("save_minutes_draft",{draftId,expectedLockVersion,payload}),structuredClone);
  queue.load(original);
  data.failMinutesSave=true;
  queue.edit(d=>editInline(d,{block:0},[textNode("終了失敗でも保持")]));
  await assert.rejects(queue.flush(),/模擬ディスク保存失敗/);
  queue.edit(d=>editInline(d,{block:0},[textNode("失敗後の追記も保持")]));
  const latest=structuredClone(queue.snapshot().payload);
  await assert.rejects(queue.flush(),/模擬ディスク保存失敗/);
  assert.equal(data.calls.filter((c:any)=>c.cmd==="save_minutes_draft").length,1);
  assert.deepEqual(queue.snapshot().draft,original);
  assert.deepEqual(queue.snapshot().payload,latest);
  assert.deepEqual(latest?.evidence,original.payload.evidence);
  assert.deepEqual(latest?.items,original.payload.items);
  assert.equal(queue.snapshot().saving,false);
  assert.equal(queue.snapshot().dirty,true);
  assert.equal(JSON.stringify(data.drafts.a),JSON.stringify(original));
  data.failMinutesSave=false;
  await queue.retry();
  await queue.flush();
  assert.equal(data.calls.filter((c:any)=>c.cmd==="save_minutes_draft").length,2);
  assert.deepEqual(queue.snapshot().payload,latest);
  assert.deepEqual(queue.snapshot().draft?.payload,latest);
  assert.equal(queue.snapshot().draft?.lockVersion,original.lockVersion+1);
  assert.equal(queue.snapshot().dirty,false);
  assert.equal(queue.snapshot().error,null);
  assert.equal(JSON.stringify(data.views.a.content),generated);
});

test("rich minutes second-save failure retains the first acknowledgement and newer pending document",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  let release!:()=>void;
  const gate=new Promise<void>(resolve=>{release=resolve;});
  let count=0;
  const queue=new DraftSaveQueue<MinutesContent,MinutesDraft>(async(draftId,expectedLockVersion,payload)=>{
    count++;
    if(count===1)await gate;
    if(count===2)throw Error("second-save failure");
    return invoke("save_minutes_draft",{draftId,expectedLockVersion,payload});
  },structuredClone);
  queue.load(original);
  queue.edit(d=>editInline(d,{block:0},[textNode("最初の保存")]));
  const first=structuredClone(queue.snapshot().payload);
  const saving=queue.flush();
  queue.edit(d=>editInline(d,{block:0},[textNode("保存待ち中の追記")]));
  const latest=structuredClone(queue.snapshot().payload);
  release();
  await assert.rejects(saving,/second-save failure/);
  assert.equal(count,2);
  assert.deepEqual(queue.snapshot().draft?.payload,first);
  assert.equal(queue.snapshot().draft?.lockVersion,original.lockVersion+1);
  assert.deepEqual(queue.snapshot().payload,latest);
  assert.deepEqual(queue.snapshot().payload?.evidence,original.payload.evidence);
  assert.equal(queue.snapshot().dirty,true);
  await assert.rejects(queue.flush(),/second-save failure/);
  assert.equal(count,2);
  await queue.retry();
  assert.equal(count,3);
  assert.deepEqual(queue.snapshot().draft?.payload,latest);
  assert.equal(queue.snapshot().draft?.lockVersion,original.lockVersion+2);
  assert.equal(queue.snapshot().dirty,false);
});

test("lost committed minutes response is reconciled only on retry without duplicating history or losing newer input",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  const generated=JSON.stringify(data.views.a.content);
  let lose=true;
  const queue=createMinutesSaveQueue(async(command,args)=>{
    const result=await invoke(command,args);
    if(command==="save_minutes_draft"&&lose){lose=false;throw Error("saved response lost");}
    return result;
  });
  queue.load(original);
  queue.edit(d=>editInline(d,{block:0},[textNode("保存済みだが応答を喪失")]));
  await assert.rejects(queue.flush(),/response lost/);
  const committed=structuredClone(data.drafts.a);
  assert.equal(committed.lockVersion,original.lockVersion+1);
  assert.equal(data.calls.filter((c:any)=>c.cmd==="get_minutes_draft").length,0);
  queue.edit(d=>editInline(d,{block:0},[textNode("応答喪失後の最新追記")]));
  const latest=structuredClone(queue.snapshot().payload);
  await assert.rejects(queue.flush(),/response lost/);
  assert.deepEqual(queue.snapshot().draft,original);
  await queue.retry();
  assert.deepEqual(Array.from(data.calls.filter((c:any)=>c.cmd==="save_minutes_draft"),(c:any)=>c.args.expectedLockVersion),[original.lockVersion,original.lockVersion+1]);
  assert.equal(data.history.a.length,2);
  assert.equal(queue.snapshot().draft?.lockVersion,original.lockVersion+2);
  assert.deepEqual(queue.snapshot().draft?.payload,latest);
  assert.deepEqual(latest?.evidence,original.payload.evidence);
  assert.deepEqual(latest?.items,original.payload.items);
  assert.equal(JSON.stringify(data.views.a.content),generated);
  assert.equal(queue.snapshot().dirty,false);
});

test("reverting to the last acknowledgement does not clear an uncertain commit or allow closing",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  let lose=true;
  const queue=createMinutesSaveQueue(async(command,args)=>{
    const result=await invoke(command,args);
    if(command==="save_minutes_draft"&&lose){lose=false;throw Error("saved response lost");}
    return result;
  });
  queue.load(original);
  queue.edit(d=>editInline(d,{block:0},[textNode("一時的な編集")]));
  await assert.rejects(queue.flush(),/response lost/);
  queue.edit(()=>structuredClone(original.payload));
  assert.equal(queue.snapshot().dirty,true);
  assert.match(queue.snapshot().error!,/response lost/);
  await assert.rejects(queue.flush(),/response lost/);
  await queue.retry();
  assert.equal(queue.snapshot().dirty,false);
  assert.deepEqual(queue.snapshot().draft?.payload,original.payload);
  assert.equal(data.history.a.length,2);
});

test("lost response with no newer edit is acknowledged on retry without another save",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  const queue=createMinutesSaveQueue(async(command,args)=>{
    const result=await invoke(command,args);
    if(command==="save_minutes_draft")throw Error("response lost");
    return result;
  });
  queue.load(original);
  queue.edit(d=>editInline(d,{block:0},[textNode("保存完了の照合")]));
  await assert.rejects(queue.flush(),/response lost/);
  await queue.retry();
  assert.equal(queue.snapshot().dirty,false);
  assert.equal(data.calls.filter((c:any)=>c.cmd==="save_minutes_draft").length,1);
  assert.equal(data.history.a.length,1);
});

for(const mismatch of ["meeting","base","id","payload","revision","missing","same-version-payload","retired-base"]){
  test(`minutes retry refuses ${mismatch} mismatch and keeps input without writing`,async()=>{
    const {data,invoke}=await fixture();
    const original=structuredClone(data.drafts.a) as MinutesDraft;
    const queue=createMinutesSaveQueue(async(command,args)=>{
      const result=await invoke(command,args);
      if(command==="save_minutes_draft")throw Error("response lost");
      return result;
    });
    queue.load(original);
    queue.edit(d=>editInline(d,{block:0},[textNode("必ず保持する編集")]));
    await assert.rejects(queue.flush(),/response lost/);
    const latest=structuredClone(queue.snapshot().payload);
    if(mismatch==="meeting")data.drafts.a.meetingId="another";
    if(mismatch==="base")data.drafts.a.baseMinutesId="another";
    if(mismatch==="id")data.drafts.a.id="another";
    if(mismatch==="payload")data.drafts.a.payload.blocks[0].content=[textNode("別の編集")];
    if(mismatch==="revision")data.drafts.a.lockVersion++;
    if(mismatch==="missing")delete data.drafts.a;
    if(mismatch==="same-version-payload")data.drafts.a.lockVersion=original.lockVersion;
    if(mismatch==="retired-base")data.current="p";
    await assert.rejects(queue.retry(),/入力を保持/);
    await assert.rejects(queue.flush(),/入力を保持/);
    assert.equal(data.calls.filter((c:any)=>c.cmd==="save_minutes_draft").length,1);
    assert.deepEqual(queue.snapshot().draft,original);
    assert.deepEqual(queue.snapshot().payload,latest);
    assert.equal(queue.snapshot().dirty,true);
    assert.equal(data.calls.find((c:any)=>c.cmd==="get_minutes_draft").args.minutesId,null);
  });
}

test("failed reconciliation read stays uncertain until another explicit retry",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  let failRead=true;
  const queue=createMinutesSaveQueue(async(command,args)=>{
    if(command==="get_minutes_draft"&&failRead)throw Error("read unavailable");
    const result=await invoke(command,args);
    if(command==="save_minutes_draft")throw Error("response lost");
    return result;
  });
  queue.load(original);
  queue.edit(d=>editInline(d,{block:0},[textNode("照合待ちの入力")]));
  await assert.rejects(queue.flush(),/response lost/);
  await assert.rejects(queue.retry(),/read unavailable/);
  queue.edit(()=>structuredClone(original.payload));
  await assert.rejects(queue.flush(),/read unavailable/);
  failRead=false;
  // The compensating write commits but again loses its response.
  await assert.rejects(queue.retry(),/response lost/);
  await queue.retry();
  assert.equal(queue.snapshot().dirty,false);
  assert.deepEqual(queue.snapshot().payload,original.payload);
  assert.equal(data.history.a.length,2);
});

test("pre-commit minutes failure retries latest input only after confirming the unchanged saved version",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  const queue=createMinutesSaveQueue(invoke);
  queue.load(original);
  data.failMinutesSave=true;
  queue.edit(d=>editInline(d,{block:0},[textNode("失敗前の入力")]));
  await assert.rejects(queue.flush(),/模擬ディスク/);
  queue.edit(d=>editInline(d,{block:0},[textNode("失敗後の最新入力")]));
  data.failMinutesSave=false;
  await queue.retry();
  assert.equal(data.history.a.length,1);
  assert.equal(queue.snapshot().draft?.lockVersion,original.lockVersion+1);
  assert.equal(queue.snapshot().dirty,false);
});

test("retry reconciliation shares one in-flight read and saves edits typed during that read",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  let release!:()=>void;
  const gate=new Promise<void>(resolve=>{release=resolve;});
  let lose=true,reads=0;
  const queue=createMinutesSaveQueue(async(command,args)=>{
    if(command==="get_minutes_draft"){reads++;await gate;}
    const result=await invoke(command,args);
    if(command==="save_minutes_draft"&&lose){lose=false;throw Error("response lost");}
    return result;
  });
  queue.load(original);
  queue.edit(d=>editInline(d,{block:0},[textNode("最初の保存")]));
  await assert.rejects(queue.flush(),/response lost/);
  const retry=queue.retry();
  assert.equal(queue.retry(),retry);
  assert.equal(queue.flush(),retry);
  assert.throws(()=>queue.load(original),/保存中/);
  queue.edit(d=>editInline(d,{block:0},[textNode("照合中の最新入力")]));
  const latest=structuredClone(queue.snapshot().payload);
  release();
  await retry;
  assert.equal(reads,1);
  assert.deepEqual(queue.snapshot().draft?.payload,latest);
  assert.equal(data.history.a.length,2);
  assert.equal(queue.snapshot().dirty,false);
});

test("another writer between reconciliation and save still fails the native version check",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  let writes=0;
  const queue=createMinutesSaveQueue(async(command,args)=>{
    if(command==="save_minutes_draft"){
      writes++;
      if(writes===2){data.drafts.a.lockVersion++;data.drafts.a.payload.blocks[0].content=[textNode("他画面の編集")];}
    }
    const result=await invoke(command,args);
    if(command==="save_minutes_draft"&&writes===1)throw Error("response lost");
    return result;
  });
  queue.load(original);
  queue.edit(d=>editInline(d,{block:0},[textNode("最初の保存")]));
  await assert.rejects(queue.flush(),/response lost/);
  queue.edit(d=>editInline(d,{block:0},[textNode("保持する最新入力")]));
  const latest=structuredClone(queue.snapshot().payload);
  await assert.rejects(queue.retry(),/stale minutes save/);
  await assert.rejects(queue.retry(),/入力を保持/);
  assert.equal(writes,2);
  assert.deepEqual(queue.snapshot().payload,latest);
  assert.equal(data.drafts.a.payload.blocks[0].content[0].text,"他画面の編集");
  assert.equal(data.history.a.length,1);
});

test("browser response-loss fault commits once then is consumed before explicit reconciliation",async()=>{
  const {data,invoke}=await fixture();
  const original=structuredClone(data.drafts.a) as MinutesDraft;
  const queue=createMinutesSaveQueue(invoke);
  queue.load(original);
  data.lostMinutesSaveResponse=true;
  queue.edit(d=>editInline(d,{block:0},[textNode("画面と同じ障害注入")]));
  await assert.rejects(queue.flush(),/保存完了後の応答喪失/);
  assert.equal(data.lostMinutesSaveResponse,false);
  assert.equal(data.history.a.length,1);
  await queue.retry();
  assert.equal(data.history.a.length,1);
  assert.equal(queue.snapshot().dirty,false);
});
