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
    start:async()=>{calls.push("start");native=snapshot();return structuredClone(native);},
    cancel:async()=>{calls.push("cancel");native=snapshot("cancelling",3);return structuredClone(native);},
    discard:async()=>{calls.push("discard");native=null;},
  };
  return {client:new CaptureTestClient(bridge),bridge,calls,set:(value:CaptureTestSnapshot|null)=>{native=value;}};
}
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
  await assert.rejects(f.client.start(request()));await assert.rejects(f.client.refresh());
  assert.equal(f.client.blocked(),true);f.set({...snapshot(),runId:"a69b7530-c4b4-40f2-b2c5-50e28244e86d"});
  await assert.rejects(f.client.refresh());await assert.rejects(f.client.start({...request(),runId:"a69b7530-c4b4-40f2-b2c5-50e28244e86d"}));
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
