import test from "node:test";
import assert from "node:assert/strict";
import {newRecordingPreparation,recordingTitle,sameRecordingPreparation,validRecordingTitle} from "../src/recording-preparation-state.ts";

const id="735027d4-a03e-43d3-830a-21e767dfaf88";
test("preparation fills a local wall-clock title without persisting or starting",()=>{
  const now=new Date(2026,9,8,9,4,23);
  assert.deepEqual(newRecordingPreparation(id,now),{id,title:"2026/10/08 09:04の会議",submitted:false});
  assert.equal(recordingTitle(new Date(2026,0,2,0,0)),"2026/01/02 00:00の会議");
});
test("title admission matches Rust Unicode character count and trims",()=>{
  assert.equal(validRecordingTitle(" \n "),false);
  assert.equal(validRecordingTitle(" 😀 "),true);
  assert.equal(validRecordingTitle("😀".repeat(200)),true);
  assert.equal(validRecordingTitle("😀".repeat(201)),false);
});
test("invalid clock and noncanonical or non-v4 IDs never produce preparation",()=>{
  for(const invalid of ["",id.toUpperCase(),id.replace("43d3","13d3"),`${id}x`])assert.throws(()=>newRecordingPreparation(invalid,new Date()));
  assert.throws(()=>newRecordingPreparation(id,new Date(NaN)));
});
test("confirmation is bound to exact local owner and title; submitted requests cannot replay",()=>{
  const expected=newRecordingPreparation(id,new Date());
  assert.equal(sameRecordingPreparation({...expected},expected),true);
  assert.equal(sameRecordingPreparation({...expected,title:` ${expected.title} `},expected),true);
  for(const current of [null,{...expected,id:"other"},{...expected,title:"別の会議"},{...expected,submitted:true}])assert.equal(sameRecordingPreparation(current,expected),false);
});
