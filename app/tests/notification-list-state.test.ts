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
test("history deletion removes active and expired events without replay from polling",()=>{
  const timers:(()=>void)[]=[];let actions=0,closed=0;
  const c=new ToastController({now:()=>0,start:run=>{timers.push(run);return run;},stop:()=>{}});
  const event={key:"event",eventId:"one",message:"通知",level:"info" as const,onClose:()=>closed++,action:{kind:"retry" as const,label:"再試行",run:()=>{actions++;}}};
  c.show(event);c.removeHistory(c.getHistorySnapshot()[0].id);
  c.resolve(event.key);c.show(event);c.record(event);
  assert.equal(c.getSnapshot().length,0);assert.equal(c.getHistorySnapshot().length,0);
  assert.equal(actions,0);assert.equal(closed,1);
  c.show({...event,eventId:"two"});const id=c.getSnapshot()[0].id;
  timers.at(-1)!();assert.equal(c.getHistorySnapshot().length,1);
  c.removeHistory(id);assert.equal(c.getHistorySnapshot().length,0);
  c.record({key:"archive",level:"warning",message:"履歴のみ"});const archived=c.getHistorySnapshot()[0];
  c.removeHistory(archived.id);c.record(archived);assert.equal(c.getHistorySnapshot().length,0);
});
test("deleting an old event never removes a newer event sharing its key",()=>{
  const c=new ToastController();c.show({key:"one",level:"error",message:"旧",eventId:"1"});
  const old=c.getHistorySnapshot()[0].id;
  c.show({key:"one",level:"error",message:"新",eventId:"2"});const current=c.getSnapshot()[0].id;
  c.removeHistory(old);c.removeHistory(old);
  assert.equal(c.getSnapshot()[0].id,current);assert.equal(c.getHistorySnapshot().length,1);
});
test("a newly occurring unversioned problem remains visible after the source resolves the old one",()=>{
  const c=new ToastController(),problem={key:"save-error",level:"error" as const,message:"保存失敗"};
  c.show(problem);c.removeHistory(c.getSnapshot()[0].id);
  c.show(problem);assert.equal(c.getSnapshot().length,0);
  c.resolve(problem.key);c.show(problem);
  assert.equal(c.getSnapshot().length,1);assert.equal(c.getHistorySnapshot().length,1);
});
