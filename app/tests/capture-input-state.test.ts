import test from "node:test";
import assert from "node:assert/strict";
import {checkedCaptureInputs,microphoneChoice,usableMicrophone,validMicrophoneUID,type CaptureInputs} from "../src/capture-input-state.ts";
const snapshot=():CaptureInputs=>({schemaVersion:1,microphonePermission:"notDetermined",selectedMicrophoneUID:null,devices:[{uid:"built-in",name:"同じ名前",isDefault:true},{uid:"usb-123",name:"同じ名前",isDefault:false}]});
test("microphone selection uses stable IDs and never replaces a missing remembered choice",()=>{
  const s=checkedCaptureInputs(snapshot());assert.equal(microphoneChoice(s,null),"built-in");
  s.selectedMicrophoneUID="missing";assert.equal(microphoneChoice(s,null),"missing");assert.equal(usableMicrophone(s,"missing"),null);
  assert.equal(microphoneChoice(s,"usb-123"),"usb-123");assert.equal(usableMicrophone(s,"usb-123")?.uid,"usb-123");assert.equal(usableMicrophone(s,"同じ名前"),null);
});
test("no default or no devices requires an explicit choice; unknown permission is not fabricated as granted",()=>{
  const s=snapshot();s.devices[0].isDefault=false;assert.equal(microphoneChoice(s,null),"");
  for(const p of ["denied","restricted"] as const){s.microphonePermission=p;assert.equal(usableMicrophone(s,"usb-123"),null);}
  for(const p of ["notDetermined","unknown","authorized"] as const){s.microphonePermission=p;assert.equal(checkedCaptureInputs(s).microphonePermission,p);assert.equal(usableMicrophone(s,"usb-123")?.uid,"usb-123");}
  s.devices=[];assert.equal(usableMicrophone(s,"usb-123"),null);
});
test("microphone catalog rejects invalid identities, duplicate defaults and malformed bounds",()=>{
  for(const uid of ["", " \t", "a\nb", "a\0b", "x".repeat(1025), "日".repeat(342)])assert.equal(validMicrophoneUID(uid),false);
  assert.equal(validMicrophoneUID("x".repeat(1024)),true);
  for(const change of [(s:CaptureInputs)=>{s.devices[1].uid=s.devices[0].uid;},(s:CaptureInputs)=>{s.devices[1].isDefault=true;},(s:CaptureInputs)=>{s.devices[0].name="";},(s:CaptureInputs)=>{s.selectedMicrophoneUID="";},(s:CaptureInputs)=>{s.devices=Array.from({length:129},(_,i)=>({uid:String(i),name:"マイク",isDefault:false}));}]){const s=snapshot();change(s);assert.throws(()=>checkedCaptureInputs(s));}
  for(const v of [null,{}, {...snapshot(),schemaVersion:2},{...snapshot(),microphonePermission:"allowed"},{...snapshot(),devices:null},{...snapshot(),selectedMicrophoneUID:undefined}])assert.throws(()=>checkedCaptureInputs(v));
});
