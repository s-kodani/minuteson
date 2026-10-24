import test from "node:test";
import assert from "node:assert/strict";
import {checkedCaptureTest,reconcileCaptureTest,validCaptureTestId,type CaptureTestSnapshot} from "../src/capture-test-state.ts";
const id="8f0b3f3a-81cd-4e4c-89a1-0147d66396e9";
const snapshot=():CaptureTestSnapshot=>({schemaVersion:1,revision:1,runId:id,phase:"preparing",busy:true,
  microphoneUid:"USB microphone",microphoneName:null,captureScope:{mode:"system",application:null},applicationName:null,
  microphone:null,system:null,cleanupPending:false,errorMessage:null});
const ready=():CaptureTestSnapshot=>({...snapshot(),revision:5,phase:"ready",busy:false,microphoneName:"USB microphone",
  microphone:{frames:40000,sampleRate:8000,channels:2,durationMs:5000,hasGaps:false},
  system:{frames:31200,sampleRate:8000,channels:2,durationMs:3900,hasGaps:true}});
test("five-second snapshot requires explicit ownership and exact complete track data",()=>{
  assert.deepEqual(checkedCaptureTest(snapshot(),id),snapshot());
  assert.deepEqual(checkedCaptureTest(ready(),id),ready());
  for(const value of [null,{}, {...snapshot(),directory:"/private/path"},{...snapshot(),meetingId:"meeting"},
    {...snapshot(),runId:id.toUpperCase()},{...snapshot(),revision:0},{...snapshot(),microphoneUid:"\0"},
    {...snapshot(),busy:false},{...ready(),busy:true},{...ready(),microphone:null,system:null},{...snapshot(),system:ready().system},
    {...ready(),cleanupPending:true},{...snapshot(),cleanupPending:true},{...snapshot(),phase:"finished"},{...snapshot(),phase:new String("preparing")},
    {...snapshot(),applicationName:"unexpected app"},{...ready(),microphone:{...ready().microphone,frames:40001}},
    {...ready(),system:{...ready().system,durationMs:5000}},{...ready(),system:{...ready().system,hasGaps:"no"}},
    {...ready(),microphone:{...ready().microphone,sampleRate:8000.5}},{...ready(),microphone:{...ready().microphone,channels:0}},
    {...ready(),microphone:{...ready().microphone,windowStartNs:123}}])assert.throws(()=>checkedCaptureTest(value,id));
  assert.throws(()=>checkedCaptureTest(snapshot(),"a69b7530-c4b4-40f2-b2c5-50e28244e86d"));
  assert.equal(validCaptureTestId(id),true);
  assert.equal(validCaptureTestId("8f0b3f3a-81cd-1e4c-89a1-0147d66396e9"),false);
});
test("phase polling can skip states but cannot revive a terminal or cancelling test",()=>{
  const prior=snapshot();
  assert.equal(reconcileCaptureTest(prior,ready(),prior).phase,"ready");
  const stopping={...prior,revision:3,phase:"cancelling" as const};
  assert.throws(()=>reconcileCaptureTest(stopping,ready(),prior));
  const stopped={...prior,revision:4,phase:"cancelled" as const,busy:false};
  assert.equal(reconcileCaptureTest(stopping,stopped,prior).phase,"cancelled");
  assert.throws(()=>reconcileCaptureTest(stopped,{...prior,revision:6},prior));
  assert.throws(()=>reconcileCaptureTest(ready(),{...prior,revision:6},prior));
});
test("stale reads do not refresh the result, and equal revisions must agree",()=>{
  const prior=ready();
  assert.equal(reconcileCaptureTest(prior,snapshot(),prior),prior);
  assert.equal(reconcileCaptureTest(prior,ready(),prior),prior);
  assert.throws(()=>reconcileCaptureTest(prior,{...prior,microphoneName:"different label"},prior));
  assert.throws(()=>reconcileCaptureTest(prior,{...prior,revision:6,system:{...prior.system!,frames:8000,durationMs:1000}},prior));
});
test("ready audio stays bound to its microphone and exact application instance",()=>{
  const prior=snapshot();
  assert.throws(()=>reconcileCaptureTest(prior,{...ready(),microphoneUid:"other mic"},prior));
  const selected={...prior,captureScope:{mode:"application" as const,application:{pid:123,bundleIdentifier:"com.example.browser",launchTimeUs:1791100000000001}}};
  assert.equal(checkedCaptureTest(selected).captureScope.mode,"application");
  assert.throws(()=>reconcileCaptureTest(selected,{...ready(),captureScope:{...selected.captureScope,application:{...selected.captureScope.application,launchTimeUs:1791100000000002}},applicationName:"same name"},selected));
  assert.throws(()=>checkedCaptureTest({...selected,captureScope:{mode:"application",application:null}}));
  const named={...selected,microphoneName:"actual input",applicationName:"actual app"};
  assert.throws(()=>reconcileCaptureTest(named,{...named,revision:2,applicationName:null},named));
  assert.throws(()=>reconcileCaptureTest(named,{...named,revision:2,microphoneName:"renamed input"},named));
});
test("terminal failure and pending cleanup remain explicit rather than ready",()=>{
  const error={...snapshot(),revision:2,phase:"error" as const,busy:false,cleanupPending:true,errorMessage:"cleanup refused"};
  assert.deepEqual(checkedCaptureTest(error),error);
  assert.throws(()=>checkedCaptureTest({...error,system:ready().system}));
  const cancellation={...error,phase:"cancelled" as const,cleanupPending:false,errorMessage:null};
  assert.deepEqual(checkedCaptureTest(cancellation),cancellation);
});

test("completed microphone preview remains available when PC audio is explicitly absent",()=>{
  const partial={...ready(),system:null};
  assert.deepEqual(checkedCaptureTest(partial),partial);
  assert.equal(reconcileCaptureTest(snapshot(),partial,snapshot()).phase,"ready");
  assert.throws(()=>checkedCaptureTest({...partial,microphone:null}));
  assert.throws(()=>checkedCaptureTest({...partial,errorMessage:"writer failed"}));
  assert.throws(()=>checkedCaptureTest({...partial,cleanupPending:true}));
  assert.throws(()=>checkedCaptureTest({...partial,system:{...ready().system!,frames:0,durationMs:0}}));
  assert.throws(()=>reconcileCaptureTest(partial,{...ready(),revision:6},snapshot()));
});

test("PC-only verified tests retain only the received track",()=>{ const value={...ready(),microphone:null}; assert.deepEqual(checkedCaptureTest(value,id),value); assert.throws(()=>checkedCaptureTest({...value,system:null},id)); });
