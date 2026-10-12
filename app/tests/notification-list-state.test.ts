import test from "node:test";
import assert from "node:assert/strict";
import { ToastController } from "../src/toast-state.ts";
test("toast expiry and dismissal retain a deduplicated notification history",()=>{
  const timers:(()=>void)[]=[];let close=0;
  const c=new ToastController({now:()=>0,start:run=>{timers.push(run);return run;},stop:()=>{}});
  const value={key:"meeting",message:"保存しました",level:"success" as const,eventId:"op-1",onClose:()=>close++};
  c.show(value);c.show(value);assert.equal(c.getHistorySnapshot().length,1);
  timers[0]();assert.equal(c.getSnapshot().length,0);assert.equal(c.getHistorySnapshot().length,1);assert.equal(close,1);
  c.show(value);assert.equal(c.getSnapshot().length,0);assert.equal(c.getHistorySnapshot().length,1);
  c.show({...value,eventId:"op-2"});assert.equal(c.getHistorySnapshot().length,2);
});
test("historical diagnostics never create popups or acknowledge native delivery",()=>{
  const c=new ToastController();let renders=0,closes=0;
  const value={key:"archive",message:"通知の保持不足",level:"warning" as const,onRendered:()=>renders++,onClose:()=>closes++};
  c.record(value);c.record(value);assert.equal(c.getSnapshot().length,0);assert.equal(c.getHistorySnapshot().length,1);
  c.resolve(value.key);assert.equal(c.getHistorySnapshot().length,1);assert.equal(renders,0);assert.equal(closes,0);
});
test("list delivery is exact-owner fenced and can render queued cards inside its own modal",()=>{
  const c=new ToastController();let rendered=0;
  for(let i=0;i<4;i++)c.show({key:`input-${i}`,message:"入力中断",level:"error",onRendered:()=>rendered++});
  const row=c.getHistorySnapshot().at(-1)!;
  c.pauseAll("modal",true);c.markRendered(row.key,row.id);assert.equal(rendered,0);
  c.markRendered(row.key,row.id+100,true);assert.equal(rendered,0);
  c.markRendered(row.key,row.id,true);c.markRendered(row.key,row.id,true);assert.equal(rendered,1);
  c.pauseAll("hidden",true);const other=c.getHistorySnapshot()[0];c.markRendered(other.key,other.id,true);assert.equal(rendered,1);
});
test("session notification history is bounded and stale actions cannot mutate an expired event",async()=>{
  const c=new ToastController();let calls=0;
  for(let i=0;i<520;i++)c.record({key:`event-${i}`,eventId:String(i),message:"通知",level:"info"});
  assert.equal(c.getHistorySnapshot().length,512);assert.equal(c.getHistorySnapshot().at(-1)?.key,"event-8");
  const run=()=>{calls++;};c.show({key:"action",message:"操作",level:"error",action:{kind:"retry",label:"再試行",run}});
  const id=c.getSnapshot()[0].id;c.dismiss("action",id);await c.act("action",id,true);assert.equal(calls,0);
  c.detachAction("action",run);assert.equal(c.getHistorySnapshot()[0].action,undefined);
});
