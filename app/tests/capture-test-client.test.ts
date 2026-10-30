import test from "node:test";
import assert from "node:assert/strict";
import {CaptureTestClient,waitForCaptureTestStop,type CaptureTestBridge} from "../src/capture-test-client.ts";
import {checkedCaptureTestPlayback,type CaptureTestRequest,type CaptureTestSnapshot} from "../src/capture-test-state.ts";
const id="8f0b3f3a-81cd-4e4c-89a1-0147d66396e9";
const request=():CaptureTestRequest=>({runId:id,microphoneUid:"USB input",captureScope:{mode:"system",application:null},expectedImport:{token:null,revision:0,busy:false}});
const snapshot=(phase:CaptureTestSnapshot["phase"]="preparing",revision=1):CaptureTestSnapshot=>({schemaVersion:1,revision,runId:id,phase,busy:["preparing","recording","checking","cancelling"].includes(phase),
  microphoneUid:"USB input",microphoneName:"USB input",captureScope:{mode:"system",application:null},applicationName:null,cleanupPending:false,errorMessage:null,
  microphone:phase==="ready"?{frames:40000,sampleRate:8000,channels:2,durationMs:5000,hasGaps:false}:null,
  system:phase==="ready"?{frames:31200,sampleRate:8000,channels:2,durationMs:3900,hasGaps:true}:null});
const deferred=<T>()=>{let resolve!:(value:T)=>void;const promise=new Promise<T>(r=>{resolve=r;});return {promise,resolve};};
function fixture(){
  let native:CaptureTestSnapshot|null=null;
  const calls:string[]=[];
  const bridge:CaptureTestBridge={
    read:async()=>{calls.push("read");return structuredClone(native);},
    reconcileStart:async()=>{calls.push("reconcile");return structuredClone(native);},
    start:async()=>{calls.push("start");native=snapshot();return structuredClone(native);},
    cancel:async()=>{calls.push("cancel");native=snapshot("cancelling",3);return structuredClone(native);},
    discard:async()=>{calls.push("discard");native=null;},
  };
  return {client:new CaptureTestClient(bridge),bridge,calls,set:(value:CaptureTestSnapshot|null)=>{native=value;}};
}

test("finishing a ready test shares release and waits for playback before deleting",async()=>{
  const f=fixture();f.set(snapshot("ready",4));await f.client.refresh();
  const playback=deferred<void>();let stopped=0;
  const stop=async(runId:string)=>{assert.equal(runId,id);stopped++;await playback.promise;};
  const first=f.client.finish(stop),second=f.client.finish(stop);
  assert.equal(first,second);assert.equal(f.client.blocked(),true);
  await new Promise(resolve=>setTimeout(resolve,0));assert.equal(stopped,1);assert.equal(f.calls.includes("discard"),false);
  playback.resolve();await first;
  assert.equal(f.calls.filter(c=>c==="discard").length,1);assert.equal(f.client.getSnapshot().snapshot,null);assert.equal(f.client.blocked(),false);
});

test("an empty finish does not mark recording priority or interrupt unrelated model preparation",async()=>{
  const f=fixture();await f.client.refresh();const blocked:boolean[]=[];
  const unsubscribe=f.client.subscribe(()=>blocked.push(f.client.blocked()));
  await f.client.finish(async()=>{throw Error("no test playback to stop");});unsubscribe();
  assert.ok(blocked.every(value=>value===false));assert.equal(f.calls.includes("discard"),false);
});

test("finishing each active phase waits for the exact process to stop",async()=>{
  for(const phase of ["preparing","recording","checking","cancelling"] as const){
    const f=fixture();f.set(snapshot(phase,2));await f.client.refresh();let paused=0;
    await f.client.finish(async()=>{assert.equal(f.client.getSnapshot().snapshot?.busy,false);},1000,async()=>{
      assert.equal(f.calls.includes("discard"),false);paused++;f.set(snapshot("cancelled",4));
    });
    assert.equal(paused,1);assert.equal(f.calls.filter(c=>c==="cancel").length,phase==="cancelling"?0:1);assert.equal(f.calls.filter(c=>c==="discard").length,1);
  }
});

test("close waits for an in-flight start and never replays a lost accepted start",async()=>{
  const f=fixture();await f.client.refresh();const receipt=deferred<CaptureTestSnapshot>();
  f.bridge.start=async()=>{f.calls.push("start");return receipt.promise;};
  const start=f.client.start(request());const finish=f.client.finish(async()=>{});
  assert.equal(f.client.getSnapshot().finishing,true);
  f.set(snapshot("ready",4));receipt.resolve(snapshot("ready",4));await start;await finish;
  assert.equal(f.calls.filter(c=>c==="start").length,1);assert.equal(f.client.getSnapshot().snapshot,null);
  const lost=fixture();await lost.client.refresh();lost.bridge.start=async()=>{lost.calls.push("start");lost.set(snapshot("ready",4));throw Error("lost reply");};
  await assert.rejects(lost.client.start(request()));await lost.client.finish(async()=>{});
  assert.equal(lost.calls.filter(c=>c==="start").length,1);assert.equal(lost.client.getSnapshot().pending,null);
});

test("finishing fences an absent pending start and leaves foreign ownership untouched",async()=>{
  const f=fixture();await f.client.refresh();f.bridge.start=async()=>{throw Error("unknown start");};
  await assert.rejects(f.client.start(request()));await f.client.finish(async()=>{throw Error("no audio to stop");});
  assert.equal(f.calls.filter(c=>c==="reconcile").length,1);assert.equal(f.calls.includes("discard"),false);
  const other=fixture();other.set(snapshot("ready",4));await other.client.refresh();
  other.set({...snapshot("ready",4),runId:"a69b7530-c4b4-40f2-b2c5-50e28244e86d"});
  await assert.rejects(other.client.finish(async()=>{}));assert.equal(other.calls.includes("discard"),false);
});

test("finish reconciles lost cancel/discard replies and permits cleanup failure retry",async()=>{
  const f=fixture();f.set(snapshot("recording",2));await f.client.refresh();
  f.bridge.cancel=async()=>{f.calls.push("cancel");f.set(snapshot("cancelled",4));throw Error("lost cancel reply");};
  f.bridge.discard=async()=>{f.calls.push("discard");f.set(null);throw Error("lost discard reply");};
  await f.client.finish(async()=>{});assert.equal(f.calls.filter(c=>c==="cancel").length,1);assert.equal(f.calls.filter(c=>c==="discard").length,1);
  const retry=fixture();retry.set(snapshot("ready",4));await retry.client.refresh();let fail=true;
  retry.bridge.discard=async()=>{if(fail)throw Error("file still open");retry.set(null);};
  await assert.rejects(retry.client.finish(async()=>{}));assert.equal(retry.client.getSnapshot().snapshot?.runId,id);assert.equal(retry.client.getSnapshot().finishing,false);
  assert.match(retry.client.getSnapshot().error,/「閉じる」/);fail=false;await retry.client.finish(async()=>{});
  assert.equal(retry.client.getSnapshot().snapshot,null);
});
test("read reconciliation never starts capture; explicit start sends once and keeps its owner",async()=>{
  const f=fixture();assert.equal(f.client.blocked(),true);await f.client.refresh();assert.equal(f.client.blocked(),false);
  await f.client.start(request());assert.equal(f.client.blocked(),true);await f.client.refresh();
  await assert.rejects(f.client.start(request()));assert.equal(f.calls.filter(c=>c==="start").length,1);
  f.set(snapshot("ready",4));await f.client.refresh();assert.equal(f.client.blocked(),false);
  await f.client.discard(id);assert.equal(f.client.getSnapshot().snapshot,null);
  await assert.rejects(f.client.start(request()));assert.equal(f.calls.filter(c=>c==="start").length,1);
});
test("lost accepted start reply is recovered by reading the same owner without replay",async()=>{
  const f=fixture();await f.client.refresh();
  f.bridge.start=async()=>{f.calls.push("start");f.set(snapshot());throw Error("lost accepted reply");};
  await assert.rejects(f.client.start(request()));assert.equal(f.client.getSnapshot().pending?.runId,id);
  assert.equal(f.client.blocked(),true);await f.client.refresh();
  assert.equal(f.client.getSnapshot().snapshot?.runId,id);assert.equal(f.client.getSnapshot().pending,null);
  assert.equal(f.calls.filter(c=>c==="start").length,1);
});
test("absent or foreign results after lost start do not authorize another capture",async()=>{
  const f=fixture();await f.client.refresh();f.bridge.start=async()=>{f.calls.push("start");throw Error("unknown acceptance");};
  f.bridge.reconcileStart=async()=>{throw Error("native reconciliation unavailable");};
  await assert.rejects(f.client.start(request()));await assert.rejects(f.client.refresh());
  assert.equal(f.client.blocked(),true);f.set({...snapshot(),runId:"a69b7530-c4b4-40f2-b2c5-50e28244e86d"});
  await assert.rejects(f.client.refresh());await assert.rejects(f.client.start({...request(),runId:"a69b7530-c4b4-40f2-b2c5-50e28244e86d"}));
  assert.equal(f.calls.filter(c=>c==="start").length,1);
});
test("a rejected start is retired by native reconciliation and no longer blocks exit or a fresh ID",async()=>{
  const f=fixture();await f.client.refresh();
  const start=f.bridge.start;
  f.bridge.start=async()=>{f.calls.push("start");throw Error("test audio recovery pending");};
  await assert.rejects(f.client.start(request()));
  assert.equal(f.client.getSnapshot().pending?.runId,id);
  assert.equal(await f.client.refresh(),null);
  assert.equal(f.client.getSnapshot().pending,null);
  assert.equal(f.client.blocked(),false);
  assert.equal(f.calls.filter(c=>c==="reconcile").length,1);
  await assert.rejects(f.client.start(request()));
  f.bridge.start=start;
  const next={...request(),runId:"a69b7530-c4b4-40f2-b2c5-50e28244e86d"};
  f.bridge.start=async()=>{f.calls.push("start");f.set({...snapshot(),runId:next.runId});return {...snapshot(),runId:next.runId};};
  await f.client.start(next);
  assert.equal(f.client.getSnapshot().snapshot?.runId,next.runId);
});
test("reconciliation of an apparently absent start recovers a late accepted run without replay",async()=>{
  const f=fixture();await f.client.refresh();
  f.bridge.start=async()=>{f.calls.push("start");throw Error("lost reply");};
  await assert.rejects(f.client.start(request()));
  f.bridge.reconcileStart=async()=>snapshot("recording",2);
  assert.equal((await f.client.refresh())?.phase,"recording");
  assert.equal(f.client.getSnapshot().pending,null);
  assert.equal(f.client.blocked(),true);
  assert.equal(f.calls.filter(c=>c==="start").length,1);
});
test("a pre-start delayed empty read cannot erase the accepted run",async()=>{
  const f=fixture();await f.client.refresh();const d=deferred<null>();f.bridge.read=()=>d.promise;
  const old=f.client.poll();await f.client.start(request());d.resolve(null);await old;
  assert.equal(f.client.getSnapshot().snapshot?.runId,id);assert.equal(f.client.getSnapshot().connected,true);
});
test("command epochs suppress polls during release and reject changed cancellation owners",async()=>{
  const f=fixture();f.set(snapshot("ready",4));await f.client.refresh();
  const d=deferred<void>();f.bridge.discard=async()=>{f.calls.push("discard");await d.promise;f.set(null);};
  const release=f.client.discard(id);await new Promise(resolve=>setTimeout(resolve,0));
  const reads=f.calls.filter(c=>c==="read").length;await f.client.poll();assert.equal(f.calls.filter(c=>c==="read").length,reads);
  d.resolve();await release;assert.equal(f.client.getSnapshot().snapshot,null);
  f.set({...snapshot(),runId:"a69b7530-c4b4-40f2-b2c5-50e28244e86d"});await f.client.refresh();
  await assert.rejects(f.client.cancel(id));assert.equal(f.calls.filter(c=>c==="cancel").length,0);
});
test("cancel stays busy until a terminal same-owner read and release waits for it",async()=>{
  const f=fixture();f.set(snapshot());await f.client.refresh();await f.client.cancel(id);
  assert.equal(f.client.blocked(),true);await assert.rejects(f.client.discard(id));assert.equal(f.calls.includes("discard"),false);
  f.set(snapshot("cancelled",4));await waitForCaptureTestStop(id,()=>f.client.refresh());await f.client.discard(id);
  assert.equal(f.client.blocked(),false);
  await assert.rejects(waitForCaptureTestStop(id,async()=>null));
});
test("lost discard receipt is reconciled only with authoritative empty state",async()=>{
  const f=fixture();f.set(snapshot("ready",4));await f.client.refresh();
  f.bridge.discard=async()=>{f.calls.push("discard");f.set(null);throw Error("lost release reply");};
  await f.client.discard(id);assert.equal(f.client.getSnapshot().snapshot,null);
  f.set(snapshot("ready",4));await f.client.refresh();f.bridge.discard=async()=>{throw Error("cleanup failed");};
  await assert.rejects(f.client.discard(id));assert.equal(f.client.getSnapshot().snapshot?.runId,id);assert.equal(f.client.blocked(),true);
});
test("preview replies match the exact physical track proof, not just a five-second duration",()=>{
  const track=snapshot("ready").system!;
  const info={token:id,sampleRate:8000,channels:2,totalFrames:31200,startUs:0,endUs:3900000,trackKind:"system",alignment:"unverified"};
  assert.deepEqual(checkedCaptureTestPlayback(info,"system",track),info);
  for(const changed of [{...info,totalFrames:40000},{...info,trackKind:"mic"},{...info,startUs:100000},{...info,endUs:5000000},{...info,sampleRate:16000},{...info,channels:1},{...info,alignment:"verified"},{...info,token:"unknown"}])assert.throws(()=>checkedCaptureTestPlayback(changed,"system",track));
});
test("concurrent same-owner cancellations and discards cannot both pass a shared fresh read",async()=>{
  for(const operation of ["cancel","discard"] as const){
    const f=fixture();f.set(snapshot(operation==="cancel"?"preparing":"ready",4));await f.client.refresh();
    const receipt=deferred<void>();
    if(operation==="cancel")f.bridge.cancel=async()=>{f.calls.push("cancel");await receipt.promise;return snapshot("cancelling",5);};
    else f.bridge.discard=async()=>{f.calls.push("discard");await receipt.promise;f.set(null);};
    const first=f.client[operation](id),second=f.client[operation](id);
    const rejected=assert.rejects(second);await new Promise(resolve=>setTimeout(resolve,0));
    assert.equal(f.calls.filter(c=>c===operation).length,1);receipt.resolve();await first;await rejected;
  }
});
