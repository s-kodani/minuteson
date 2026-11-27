import test from "node:test";
import assert from "node:assert/strict";
import {InputLevelSession,checkedInputLevels,type LevelTrack} from "../src/input-level-state.ts";
import {RecoverableRead,sharedClock,type RecoverableAnomaly} from "../src/recoverable-read.ts";
import {RecordingInputSession,checkedInputStatus,type MicrophoneStatus} from "../src/recording-input-state.ts";
import {InputJournalFailures} from "../src/input-journal-state.ts";
const id="735027d4-a03e-43d3-830a-21e767dfaf88";
const measured:LevelTrack={state:"measured",windowFrames:1024,ageMs:20,rms:.5,peak:1};
const waiting:LevelTrack={state:"waiting",windowFrames:null,ageMs:null,rms:null,peak:null};
const level=(revision:number)=>({schemaVersion:1 as const,ownerId:id,revision,reportAgeMs:0,microphone:measured,system:waiting});
const binding=():MicrophoneStatus=>({state:"connected",sourceEpoch:1,interruptions:0,reason:null});
const status=(revision:number)=>({schemaVersion:4 as const,ownerId:id,revision,reportAgeMs:0,microphone:binding(),system:binding(),notices:[],noticeLoss:{microphone:0,system:0}});
const journalPersistence=(r:number):JournalPersistence=>({requestedRevision:r,committedRevision:r,state:"confirmed",closed:false});

test("hung reads stay at one in-flight before the timeout, then retry with backoff and a hard ceiling",async()=>{
  let mono=1000,wall=10000;const anomalies:RecoverableAnomaly[]=[];const pending:Array<(v:unknown)=>void>=[];
  const clock=sharedClock(mono,wall);const session=new InputLevelSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,graceMs:0,
    revision:v=>{try{return {revision:checkedInputLevels(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>new Promise(resolve=>pending.push(resolve)),
    onAnomaly:kind=>anomalies.push(kind),
  });
  poll.poll();assert.equal(poll.inFlightCount(),1);
  poll.poll();assert.equal(poll.inFlightCount(),1);
  mono+=3000;for(let i=0;i<3;i++)poll.advance();
  assert.ok(anomalies.includes("hung"));
  for(let i=0;i<2;i++){poll.poll();mono+=3000;for(let j=0;j<3;j++)poll.advance();}
  poll.poll();assert.equal(poll.inFlightCount(),1);
  poll.poll();assert.equal(poll.inFlightCount(),1);
  while(pending.length)pending.shift()?.(level(1));
  await Promise.resolve();poll.dispose();
});

test("late and reversed responses are ignored; clock reset waits for newer revisions",async()=>{
  let mono=1000,wall=10000;const pending:Array<(v:unknown)=>void>=[];
  const clock=sharedClock(mono,wall);const session=new InputLevelSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,
    revision:v=>{try{return {revision:checkedInputLevels(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>new Promise(resolve=>pending.push(resolve)),
  });
  poll.poll();pending.shift()?.(level(2));await Promise.resolve();
  assert.equal(poll.revision(),2);
  poll.poll();pending.shift()?.(level(1));await Promise.resolve();
  assert.equal(poll.revision(),2);
  mono=900;wall=9000;poll.advance();
  assert.equal(session.read(clock.read(mono,wall)).microphone.state,"unavailable");
  poll.poll();pending.shift()?.(level(2));await Promise.resolve();
  assert.equal(poll.revision(),null);
  poll.poll();pending.shift()?.(level(3));await Promise.resolve();
  assert.equal(poll.revision(),3);
  poll.poll();pending.shift()?.(level(2));await Promise.resolve();
  assert.equal(poll.revision(),3);poll.dispose();
});

test("advancing publications whose tracks are still waiting never raise anomalies",async()=>{
  // A silent PC audio track legitimately stays "waiting"; only a stuck
  // publication revision is a stall.
  let mono=1000,wall=10000,revision=0;const anomalies:RecoverableAnomaly[]=[];
  const clock=sharedClock(mono,wall);const session=new InputLevelSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,graceMs:0,
    revision:v=>{try{const r=checkedInputLevels(v,id);return {revision:r.revision,reportAgeMs:r.reportAgeMs};}catch{return null;}},
    invoke:()=>Promise.resolve({...level(++revision),microphone:waiting}),
    onAnomaly:kind=>anomalies.push(kind),
  });
  for(let i=0;i<60;i++){poll.poll();await Promise.resolve();await Promise.resolve();mono+=250;wall+=250;poll.advance();}
  assert.ok(!anomalies.includes("stalled"));poll.dispose();
});

test("connection status recovers after clock reversal with newer reports",async()=>{
  let mono=1000,wall=10000,current=1;const clock=sharedClock(mono,wall);
  const session=new RecordingInputSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,
    revision:v=>{try{return {revision:checkedInputStatus(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>Promise.resolve(status(current)),
  });
  poll.poll();await Promise.resolve();
  assert.equal(poll.read().microphone.state,"connected");
  mono=900;wall=9000;poll.reverseClock();
  assert.equal(poll.read().microphone.state,"unknown");
  current=2;poll.poll();await Promise.resolve();
  assert.equal(poll.read().microphone.state,"unknown");
  current=3;poll.poll();await Promise.resolve();
  assert.equal(poll.read().microphone.state,"connected");
  current=1;poll.poll();await Promise.resolve();
  assert.equal(poll.read().microphone.state,"connected");poll.dispose();
});

test("one failed read clears display but polling continues and the next success recovers",async()=>{
  let mono=1000,wall=10000,shouldFail=true;const anomalies:RecoverableAnomaly[]=[];
  const clock=sharedClock(mono,wall);const session=new InputLevelSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,graceMs:0,
    revision:v=>{try{return {revision:checkedInputLevels(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>shouldFail?Promise.reject(new Error("rpc down")):Promise.resolve(level(2)),
    onAnomaly:(kind,detail)=>{anomalies.push(kind);if(kind==="recovered")assert.ok((detail.kinds as string[]).length);},
  });
  poll.poll();await Promise.resolve();await Promise.resolve();
  assert.equal(poll.failed,true);
  assert.equal(poll.read().microphone.state,"unavailable");
  assert.ok(!anomalies.includes("failed"));
  for(let i=0;i<4;i++){mono+=1000;poll.tick();await Promise.resolve();}
  assert.ok(anomalies.includes("failed"));
  shouldFail=false;
  poll.tick();await Promise.resolve();
  assert.equal(poll.failed,false);
  assert.equal(poll.read().microphone.state,"measured");
  assert.ok(anomalies.includes("recovered"));poll.dispose();
});

test("stall and recovery each fire once",async()=>{
  let mono=1000,wall=10000;let fresh=false;const events:string[]=[];
  const clock=sharedClock(mono,wall);const session=new InputLevelSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,graceMs:0,
    revision:v=>{try{return {revision:checkedInputLevels(v,id).revision,reportAgeMs:(v as {reportAgeMs:number}).reportAgeMs};}catch{return null;}},
    invoke:()=>Promise.resolve(fresh?level(2):{...level(1),reportAgeMs:5000}),
    onAnomaly:(kind,detail)=>events.push(kind==="recovered"?`recovered:${(detail.kinds as string[]).join(",")}`:kind),
  });
  for(let i=0;i<4;i++){poll.poll();await Promise.resolve();mono+=1500;poll.advance();}
  assert.equal(events.filter(e=>e==="stalled").length,1);
  fresh=true;
  poll.poll();await Promise.resolve();
  assert.equal(events.filter(e=>e.startsWith("recovered")).length,1);poll.dispose();
});

test("clock reversal keeps pre-reset flights in the slot budget",async()=>{
  let mono=1000,wall=10000;const pending:Array<(v:unknown)=>void>=[];
  const clock=sharedClock(mono,wall);const session=new InputLevelSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,
    revision:v=>{try{return {revision:checkedInputLevels(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>new Promise(resolve=>pending.push(resolve)),
  });
  poll.poll();assert.equal(poll.inFlightCount(),1);
  mono=900;wall=9000;poll.reverseClock();
  assert.equal(poll.inFlightCount(),1);
  poll.poll();assert.equal(poll.inFlightCount(),2);
  pending.shift()?.(level(1));await Promise.resolve();
  assert.equal(poll.revision(),null);poll.dispose();
});

test("journal write failures stay visible across clock reversal",()=>{
  const failures=new InputJournalFailures();
  const view={state:"failed" as const,requestedRevision:2,committedRevision:1};
  assert.equal(failures.read(view,"recording",1000),"failed");
  failures.read(view,"recording",1000);
  assert.equal(failures.read(view,"recording",1000),"failed");
});

test("two consecutive clock reversals stay bounded",async()=>{
  let mono=1000,wall=10000,rev=1;const clock=sharedClock(mono,wall);
  const session=new InputLevelSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,
    revision:v=>{try{return {revision:checkedInputLevels(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>Promise.resolve(level(rev++)),
  });
  poll.poll();await Promise.resolve();
  mono=900;wall=9000;poll.reverseClock();
  mono=800;wall=8000;poll.reverseClock();
  assert.equal(poll.revision(),null);
  poll.poll();await Promise.resolve();
  assert.equal(poll.revision(),null);
  poll.poll();await Promise.resolve();
  assert.equal(poll.revision(),3);poll.dispose();
});

test("startup grace suppresses immediate stall reports after sleep",async()=>{
  let mono=1000,wall=10000;const anomalies:RecoverableAnomaly[]=[];
  const clock=sharedClock(mono,wall);const session=new InputLevelSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,
    revision:v=>{try{return {revision:checkedInputLevels(v,id).revision,reportAgeMs:5000};}catch{return null;}},
    invoke:()=>Promise.resolve(level(1)),
    onAnomaly:kind=>anomalies.push(kind),
  });
  for(let i=0;i<3;i++){poll.poll();await Promise.resolve();mono+=2000;poll.advance();}
  assert.ok(!anomalies.includes("stalled"));
  mono+=20000;
  for(let i=0;i<3;i++){poll.poll();await Promise.resolve();mono+=1500;poll.advance();}
  assert.ok(anomalies.includes("stalled"));poll.dispose();
});

test("each read result reaches onRead at once: the reply error, then null on recovery",async()=>{
  let mono=1000,wall=10000,fail=true;const results:unknown[]=[];
  const clock=sharedClock(mono,wall);const session=new RecordingInputSession();session.reset(id);
  const poll=new RecoverableRead(id,session,clock,{mono:()=>mono,wall:()=>wall,
    revision:v=>{try{return {revision:checkedInputStatus(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>fail?Promise.reject(Error("模擬マイク状態未確認")):Promise.resolve(status(1)),
    onRead:error=>results.push(error),
  });
  poll.poll();await Promise.resolve();await Promise.resolve();
  assert.equal((results[0] as Error).message,"模擬マイク状態未確認");
  fail=false;poll.poll();await Promise.resolve();await Promise.resolve();
  assert.equal(results[1],null);assert.equal(poll.read().microphone.state,"connected");
  poll.dispose();
});
