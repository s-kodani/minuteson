import test from "node:test";
import assert from "node:assert/strict";
import {createInputLevelPoller,type LevelDisplay} from "../src/input-level-poller.ts";
import {InputDiagnosticLog} from "../src/input-diagnostic-log.ts";
import type {RecoverableAnomaly} from "../src/recoverable-read.ts";
const id="735027d4-a03e-43d3-830a-21e767dfaf88";
const flush=async()=>{for(let i=0;i<3;i++)await Promise.resolve();};
const level=(revision:number,mic="measured" as const)=>({
  schemaVersion:1 as const,ownerId:id,revision,reportAgeMs:0,
  microphone:{state:mic,windowFrames:1024,ageMs:0,rms:.5,peak:1},
  system:{state:"waiting" as const,windowFrames:null,ageMs:null,rms:null,peak:null},
});

test("poller exposes tick without setInterval",async()=>{
  const updates:LevelDisplay[]=[];
  const {tick,dispose}=createInputLevelPoller({owner:id,kind:"recording",mono:()=>1000,wall:()=>10000,
    invoke:()=>Promise.resolve(level(1)),
    onUpdate:v=>updates.push(v),
  });
  assert.equal(typeof tick,"function");
  await flush();
  tick();
  assert.equal(updates.at(-1)?.revision,1);
  dispose();
});

test("one in-flight read until hung timeout then retries",async()=>{
  let mono=1000,wall=10000;const pending:Array<(v:unknown)=>void>=[];const updates:LevelDisplay[]=[];
  const {tick,dispose}=createInputLevelPoller({owner:id,kind:"recording",mono:()=>mono,wall:()=>wall,
    invoke:()=>new Promise(resolve=>pending.push(resolve)),
    onUpdate:v=>updates.push(v),
  });
  tick();tick();
  assert.equal(pending.length,1);
  mono+=3000;
  for(let i=0;i<3;i++)tick();
  tick();
  assert.equal(pending.length,2);
  pending.pop()?.(level(1));await flush();tick();
  assert.equal(updates.at(-1)?.microphone.state,"measured");
  dispose();
});

test("late responses never change the displayed revision",async()=>{
  let mono=1000,wall=10000,rev=1;const pending:Array<(v:unknown)=>void>=[];const updates:LevelDisplay[]=[];
  const {tick,dispose}=createInputLevelPoller({owner:id,kind:"recording",mono:()=>mono,wall:()=>wall,
    invoke:()=>new Promise(resolve=>pending.push(resolve)),
    onUpdate:v=>updates.push(v),
  });
  tick();pending.shift()?.(level(rev++));await Promise.resolve();tick();
  tick();pending.shift()?.(level(rev-2));await Promise.resolve();tick();
  assert.equal(updates.at(-1)?.revision,1);
  dispose();
});

test("failed read shows unavailable then recovers on the next success",async()=>{
  let mono=1000,wall=10000,fail=true;const updates:LevelDisplay[]=[];
  const diagnostic=new InputDiagnosticLog(()=>Promise.resolve());
  const {tick,dispose}=createInputLevelPoller({owner:id,kind:"recording",mono:()=>mono,wall:()=>wall,diagnostic,
    invoke:()=>fail?Promise.reject(new Error("down")):Promise.resolve(level(2)),
    onUpdate:v=>updates.push(v),
  });
  tick();await flush();
  assert.equal(updates.at(-1)?.microphone.state,"unavailable");
  fail=false;tick();await flush();tick();
  assert.equal(updates.at(-1)?.microphone.state,"measured");
  dispose();
});

test("window label separates main and recorder diagnostic keys",()=>{
  const sent:object[]=[];
  const diagnostic=new InputDiagnosticLog(payload=>{sent.push(payload);return Promise.resolve();});
  diagnostic.bindWindow("recorder");
  diagnostic.reportAnomaly("live-levels",id,"stalled",{idleMs:3000});
  diagnostic.bindWindow("main");
  diagnostic.reportAnomaly("live-levels",id,"stalled",{idleMs:3000});
  assert.equal(sent.length,2);
});
