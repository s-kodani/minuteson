import test from "node:test";
import assert from "node:assert/strict";
import { runInNewContext } from "node:vm";
import { DraftSaveQueue } from "../src/draft-save.ts";
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
