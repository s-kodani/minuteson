import test from "node:test";
import assert from "node:assert/strict";
import {ArchivedInputController} from "../src/archived-input-controller.ts";
import {ArchivedInputScheduler} from "../src/archived-input-scheduler.ts";
import {checkedArchivedInputRows,type ArchivedInputRow} from "../src/archived-input-state.ts";
import {InputLevelClock} from "../src/input-level-state.ts";

const id=(i:number)=>`${i.toString(16).padStart(8,"0")}-aaaa-4aaa-8aaa-aaaaaaaaaaaa`;
const row=(i:number,active=true):ArchivedInputRow=>({meetingId:id(1000+i),sessionId:id(i),title:`会議${i}`,issue:null,historySaveIssue:null,
  batch:{schemaVersion:1,ownerId:id(i),revision:1,historyIssue:null,noticeLoss:{microphone:0,system:0},notices:active?[{
    id:`${id(i)}:microphone:1:interrupted`,input:"microphone",target:"main",kind:"interrupted",interruptions:1,recipientId:null,
    delivery:{sequence:1,generation:1,leaseAgeMs:null,rendered:false,receiveId:null,status:{state:"disconnected",sourceEpoch:1,interruptions:1,reason:"device_disconnected"}},
  }]:[]}});
const page=(rows:ArchivedInputRow[],next:string|null=null)=>({schemaVersion:3,recordings:rows,nextCursor:next});
const ready=(value:unknown)=>({paused:false,value});
const flush=async()=>{for(let i=0;i<12;i++)await Promise.resolve();};
function harness(initial:ArchivedInputRow[]=[]){
  let at=0,serial=0,wallShift=0,clockAvailable=true;const timers=new Map<number,{at:number;run:()=>void}>();
  const wall=()=>clockAvailable?100_000+at+wallShift:NaN;
  let clock=new InputLevelClock(at,wall());
  const calls:{at:number;request:any}[]=[],errors:string[]=[];
  const c=new ArchivedInputController(id(999));
  let rows=initial,hold:((v:unknown)=>void)|null=null,held=false,fail=false;
  const s=new ArchivedInputScheduler(c,{now:()=>clock.read(at,wall()),reset:()=>{clock=new InputLevelClock(at,wall());return clock.read(at,wall())===0;},
    start:(run,delay)=>{const id=++serial;timers.set(id,{at:at+delay,run});return id;},stop:id=>{timers.delete(id as number);}},async request=>{
    calls.push({at,request});if(held)return new Promise(resolve=>{hold=resolve;});if(fail)throw Error("read failed");
    if(request.kind==="page"){
      const found=rows.filter(r=>request.after===null||r.sessionId>request.after).slice().sort((a,b)=>a.sessionId.localeCompare(b.sessionId));
      return ready(page(found.slice(0,32),found.length>32?found[31].sessionId:null));
    }
    const recordings=rows.filter(r=>request.targets.some(t=>t.sessionId===r.sessionId&&t.meetingId===r.meetingId));
    return ready({schemaVersion:1,recordings,missingSessionIds:request.targets.filter(t=>!recordings.some(r=>r.sessionId===t.sessionId)).map(t=>t.sessionId)});
  },(_reading,error)=>{errors.push(error);});
  async function advance(ms:number){const end=at+ms;await flush();let runs=0;while(true){
    const next=[...timers].filter(([,t])=>t.at<=end).sort((a,b)=>a[1].at-b[1].at)[0];if(!next)break;
    assert.ok(++runs<2000,"timer storm");at=next[1].at;timers.delete(next[0]);next[1].run();await flush();
  }at=end;await flush();}
  return {s,c,calls,errors,advance,setRows:(value:ArchivedInputRow[])=>{rows=value;},setFail:(value:boolean)=>{fail=value;},hold:()=>{held=true;},release:(value:unknown)=>{held=false;hold!(value);},timers,
    shiftWall:(delta:number)=>{wallShift+=delta;},setClockAvailable:(available:boolean)=>{clockAvailable=available;}};
}

test("idle discovery sleeps for 60 seconds; diagnostics never start targeted reads",async()=>{
  const h=harness([row(1,false)]);h.s.start(false);await h.advance(0);assert.equal(h.calls.length,1);
  assert.equal(h.c.needsPaint(),false);await h.advance(59_999);assert.equal(h.calls.length,1);
  await h.advance(1);assert.equal(h.calls.length,2);assert.ok(h.calls.every(c=>c.request.kind==="page"));h.s.dispose();assert.equal(h.timers.size,0);
});
test("only unfinished owners are read and polling stops after authoritative empty readback",async()=>{
  const h=harness([row(1),row(2,false)]);h.s.start(false);await h.advance(1500);
  assert.equal(h.calls.filter(c=>c.request.kind==="page").length,1);
  assert.deepEqual(h.calls.slice(1).map(c=>c.request.targets.map((t:any)=>t.sessionId)),[[id(1)],[id(1)],[id(1)]]);
  h.setRows([row(1,false),row(2,false)]);await h.advance(500);const count=h.calls.length;await h.advance(20_000);assert.equal(h.calls.length,count);
});
test("events coalesce and invalidation during a hung read is replayed without overlap",async()=>{
  const h=harness();h.hold();h.s.start(false);await h.advance(0);
  h.s.invalidate();await h.advance(100);h.s.invalidate();await h.advance(2000);assert.equal(h.calls.length,1);
  h.release(ready(page([])));await h.advance(0);assert.equal(h.calls.length,2);assert.equal(h.calls[1].request.after,null);
  h.s.invalidate();await h.advance(100);h.s.invalidate();await h.advance(149);assert.equal(h.calls.length,2);await h.advance(1);assert.equal(h.calls.length,3);
});
test("pause fences late reads; resume starts fresh without waiting for the idle deadline",async()=>{
  const h=harness();h.hold();h.s.start(false);await h.advance(0);h.s.setBlocked(true);h.release(ready(page([row(1)])));await h.advance(10_000);
  assert.equal(h.c.targets().length,0);assert.equal(h.calls.length,1);
  h.s.setBlocked(false);h.setRows([row(2,false)]);await h.advance(0);assert.equal(h.calls.length,2);assert.equal(h.c.paint(10_000).views[0].sessionId,id(2));
});
test("discovery wraps, and targeted refresh cannot remove unseen owners or move the cursor",async()=>{
  const h=harness(Array.from({length:40},(_,i)=>row(i+1,i===0)));h.s.start(false);await h.advance(600);
  assert.equal(h.calls.filter(c=>c.request.kind==="page").length,2);assert.equal(h.c.after(),null);
  assert.equal(h.c.paint(600).views.length,40);
  h.setRows([row(0),...Array.from({length:40},(_,i)=>row(i+1,i===0))]);h.s.invalidate();await h.advance(300);
  assert.ok(h.c.targets().some(t=>t.sessionId===id(0)));
});
test("target batches rotate fairly and alternate with discovery pages",async()=>{
  const h=harness(Array.from({length:100},(_,i)=>row(i+1)));h.s.start(false);await h.advance(1800);
  const requests=h.calls.filter(c=>c.request.kind==="rows");assert.ok(requests.every(c=>c.request.targets.length<=32));
  assert.equal(new Set(requests.flatMap(c=>c.request.targets.map((t:any)=>t.sessionId))).size,100);
  assert.equal(h.calls.filter(c=>c.request.kind==="page").length,4);
});
test("read failure backs off per target without hiding unrelated owners; manual retry bypasses delay",async()=>{
  const h=harness([row(1)]);h.s.start(false);await h.advance(0);h.setFail(true);await h.advance(500);
  assert.equal(h.errors.at(-1)?.includes("確認できません"),true);assert.equal(h.c.paint(500).views[0].unavailable,true);
  await h.advance(999);assert.equal(h.calls.length,2);await h.advance(1);assert.equal(h.calls.length,3);
  await h.advance(1999);assert.equal(h.calls.length,3);await h.advance(1);assert.equal(h.calls.length,4);
  h.setFail(false);h.s.invalidate(true);await h.advance(0);assert.ok(h.calls.length>4);assert.equal(h.errors.at(-1),"");
});
test("discovery failure has bounded exponential retry and paused replies do not erase state",async()=>{
  const h=harness();h.setFail(true);h.s.start(false);await h.advance(0);await h.advance(127_000);
  assert.deepEqual(h.calls.map(c=>c.at),[0,1000,3000,7000,15000,31000,63000,123000]);
  h.s.dispose();
  const paused=harness([row(1)]);paused.s.start(false);await paused.advance(0);paused.hold();await paused.advance(500);
  paused.release({paused:true,value:null});await paused.advance(0);assert.equal(paused.c.targets().length,1);assert.equal(paused.c.paint(500).views[0].unavailable,false);
});
test("selected response requires exact coverage, unique owners, matching meetings and bounded requests",()=>{
  const target={meetingId:id(1001),sessionId:id(1)},valid={schemaVersion:1,recordings:[row(1)],missingSessionIds:[]};
  assert.deepEqual(checkedArchivedInputRows(valid,[target]),valid);
  for(const value of [{...valid,recordings:[]},{...valid,missingSessionIds:[id(1)]},{...valid,recordings:[row(2)]},{...valid,extra:true},
    {...valid,recordings:[{...row(1),meetingId:id(444)}]},{...valid,recordings:[],missingSessionIds:[id(2)]}])assert.throws(()=>checkedArchivedInputRows(value,[target]));
  assert.throws(()=>checkedArchivedInputRows(valid,[target,target]));assert.throws(()=>checkedArchivedInputRows(valid,[]));
});

test("one unavailable row backs off while an unrelated owner keeps fresh delivery",async()=>{
  const h=harness([row(1),row(2)]);h.s.start(false);await h.advance(0);
  h.setRows([{...row(1),batch:null,issue:"unavailable"},row(2)]);await h.advance(500);
  const views=h.c.paint(500).views;assert.equal(views[0].unavailable,true);assert.equal(views[1].unavailable,false);
  await h.advance(500);assert.deepEqual(h.calls.at(-1)?.request.targets.map((t:any)=>t.sessionId),[id(2)]);
  await h.advance(500);assert.ok(h.calls.at(-1)?.request.targets.some((t:any)=>t.sessionId===id(1)));
});
test("thousands of completed histories stay bounded without starting delivery polling",async()=>{
  const h=harness(Array.from({length:1000},(_,i)=>row(i+1,false)));h.s.start(false);await h.advance(2000);
  assert.equal(h.calls.length,32);assert.ok(h.calls.every(c=>c.request.kind==="page"));assert.equal(h.c.paint(2000).views.length,128);
  await h.advance(30_000);assert.equal(h.calls.length,32);
});
test("target disappearance fences outstanding callbacks without changing discovery position",()=>{
  const c=new ArchivedInputController(id(999));const rows=Array.from({length:32},(_,i)=>row(i+1,i===0));
  c.acceptPage(page(rows,id(32)),null,0);const request=c.paint(0).attempts[0],targets=c.targets();
  c.acceptRows({schemaVersion:1,recordings:[],missingSessionIds:[id(1)]},targets,1);
  assert.equal(c.after(),id(32));assert.equal(c.targets().length,0);
  assert.equal(c.complete(request,{},2),false);
});

test("startup admission stays idle until unblocked, and disposal rejects held responses",async()=>{
  const h=harness([row(1)]);h.s.start(true);await h.advance(120_000);assert.equal(h.calls.length,0);
  h.hold();h.s.setBlocked(false);await h.advance(0);assert.equal(h.calls.length,1);
  h.s.dispose();h.release(ready(page([row(1)])));await h.advance(10_000);
  assert.equal(h.c.targets().length,0);assert.equal(h.timers.size,0);
});
test("a UI meeting list that lags discovery is not proof of removal",()=>{
  const c=new ArchivedInputController(id(999));c.acceptPage(page([row(1),row(2)]),null,0);
  c.removeMeetings(new Set([id(1001)]));
  assert.deepEqual(c.targets().map(t=>t.sessionId),[id(2)]);
});

test("wall clock rollback fences a pending read and preserves pending delivery across fresh discovery",async()=>{
  const h=harness([row(1)]);h.s.start(false);await h.advance(0);
  assert.equal(h.c.paint(h.s.now()).attempts[0].attempt.action,"receive");
  h.hold();await h.advance(500);
  assert.equal(h.calls.at(-1)?.request.kind,"rows");
  h.shiftWall(-60_000);
  h.setRows([row(1),row(2)]);
  h.release(ready({schemaVersion:1,recordings:[],missingSessionIds:[id(1)]}));
  await h.advance(0);
  assert.deepEqual(h.calls[2]?.request,{kind:"page",after:null});
  const {views,attempts}=h.c.paint(h.s.now());
  assert.equal(views.length,2);
  assert.ok(views.every(view=>view.checked));
  assert.deepEqual(attempts.map(a=>a.attempt.sessionId),[id(2)]);
  assert.ok(h.errors.every(error=>error===""));
});

test("an unavailable clock retries with one timer and resumes when sampling works",async()=>{
  const h=harness();h.setClockAvailable(false);h.s.start(false);
  await h.advance(500);h.s.sync();h.s.invalidate(true);
  assert.deepEqual([...h.timers.values()].map(t=>t.at),[1000]);
  await h.advance(500);
  assert.deepEqual([...h.timers.values()].map(t=>t.at),[2000]);
  assert.equal(h.calls.length,0);
  h.setClockAvailable(true);await h.advance(1000);
  assert.deepEqual(h.calls,[{at:2000,request:{kind:"page",after:null}}]);
  assert.ok(h.errors.every(error=>error===""));
  h.s.dispose();assert.equal(h.timers.size,0);
});
