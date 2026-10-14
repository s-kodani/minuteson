import test from "node:test";
import assert from "node:assert/strict";
import {checkedMicrophoneSelectionStatus,microphoneFallbackMessage} from "../src/microphone-selection-state.ts";

const owner="735027d4-a03e-43d3-830a-21e767dfaf88";
const fallback=()=>({schemaVersion:1,ownerId:owner,revision:1,requestRevision:0,mode:"automatic",deviceUID:"built-in",source:"systemDefault",fallbackReason:"application_input_unavailable"});
test("only an owned automatic fallback becomes a user notification",()=>{
  const status=checkedMicrophoneSelectionStatus(fallback(),owner);
  assert.match(microphoneFallbackMessage(status)??"",/OSの既定入力/);
  assert.equal(microphoneFallbackMessage(checkedMicrophoneSelectionStatus({...fallback(),fallbackReason:"mon_direct_recording"},owner)),null);
  assert.equal(microphoneFallbackMessage(checkedMicrophoneSelectionStatus({...fallback(),source:"application",fallbackReason:null},owner)),null);
});
test("microphone source status rejects stale ownership and fabricated sources",()=>{
  for(const [field,value] of [["ownerId","../other"],["revision",0],["source","other"],["fallbackReason","arbitrary"],["deviceUID","bad\nuid"],["requestRevision",-1]] as const){
    assert.throws(()=>checkedMicrophoneSelectionStatus({...fallback(),[field]:value},owner));
  }
  assert.throws(()=>checkedMicrophoneSelectionStatus({...fallback(),extra:true},owner));
});
test("a fixed-device switch can remain pending without claiming the old device is fixed",()=>{
  const pending=checkedMicrophoneSelectionStatus({...fallback(),mode:"fixed",source:"pending",fallbackReason:null,requestRevision:1},owner);
  assert.equal(pending.source,"pending");
  assert.equal(microphoneFallbackMessage(pending),null);
});
