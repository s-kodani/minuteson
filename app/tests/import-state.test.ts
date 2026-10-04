import test from "node:test";
import assert from "node:assert/strict";
import { acceptImport, canStartImport, droppedAudio, importLabel, type ImportSnapshot } from "../src/import-state.ts";
const ready:ImportSnapshot={revision:10,phase:"ready",busy:false,recordingBlocked:false,token:crypto.randomUUID(),fileName:"会議.wav",info:{schemaVersion:1,detectedFormat:"wav",codec:"pcm",sampleRate:16000,channels:1,frames:64000,durationUs:4000000},sourceBytes:128044,requiredBytes:200000000,availableBytes:300000000,completed:0,saved:null,errorMessage:null};
test("only confirmed idle imports with capacity, valid title and vocabulary can start",()=>{
  assert.ok(canStartImport(ready,"会議",[],0,false));
  for(const patch of [{phase:"inspecting"},{phase:"failed"},{busy:true},{recordingBlocked:true},{token:null},{info:null},{availableBytes:null},{requiredBytes:null},{availableBytes:1}])assert.equal(canStartImport({...ready,...patch},"会議",[],0,false),false);
  assert.equal(canStartImport(ready,"会議",[],0,true),false);
  for(const title of [" ","字".repeat(201)])assert.equal(canStartImport(ready,title,[],0,false),false);
  for(const version of [null,-1,0.5])assert.equal(canStartImport(ready,"会議",[],version,false),false);
  assert.equal(canStartImport(ready,"会議",[{id:crypto.randomUUID(),term:"",reading:null,note:null,enabled:true}],0,false),false);
});
test("native revisions preserve newer terminal states after lost or reordered responses",()=>{
  const saved={...ready,revision:20,phase:"saved",saved:{id:"import",meetingId:"meeting",state:"ready"}};
  assert.equal(acceptImport(saved,ready),saved);
  assert.equal(acceptImport(ready,saved),saved);
  assert.equal(acceptImport(null,ready),ready);
  assert.equal(importLabel("saved"),"音声を保存しました");
});
test("drop takes exactly one local candidate and leaves actual format checking to native code",()=>{
  assert.equal(droppedAudio(["/local/audio/wrong-extension.data"]),"/local/audio/wrong-extension.data");
  for(const paths of [[],[""],["/a.wav","/b.wav"]])assert.throws(()=>droppedAudio(paths),/1件/);
});
