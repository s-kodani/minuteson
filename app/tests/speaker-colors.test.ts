import test from "node:test";
import assert from "node:assert/strict";
import { speakerPalette,speakerTone } from "../src/speaker-colors.ts";
import { transcriptTrackKinds } from "../src/transcript-track-kinds.ts";

test("speaker colors depend on stable meeting and speaker IDs, not names, order, visibility or array indexes",()=>{
  const before=new Map(["a","b","manual:123","日本語"].map(id=>[id,speakerTone("meeting",id)]));
  for(const id of ["日本語","manual:123","b","a"])assert.equal(speakerTone("meeting",id),before.get(id));
  for(let index=0;index<1000;index++)assert.ok(speakerTone("meeting",String(index))>=0&&speakerTone("meeting",String(index))<speakerPalette.length);
  assert.equal(speakerTone("meeting","speaker-a"),speakerTone("meeting","speaker-a"));
  assert.notEqual(speakerTone("ab","c"),speakerTone("a","bc"));
});

test("every light and dark speaker label pair exceeds the 4.5 contrast design target",()=>{
  const luminance=(hex:string)=>{
    const values=[1,3,5].map(index=>parseInt(hex.slice(index,index+2),16)/255)
      .map(value=>value<=0.04045?value/12.92:((value+0.055)/1.055)**2.4);
    return values[0]*0.2126+values[1]*0.7152+values[2]*0.0722;
  };
  for(const colors of speakerPalette)for(const [fg,bg] of [[colors[0],colors[1]],[colors[2],colors[3]]]){
    const a=luminance(fg),b=luminance(bg),ratio=(Math.max(a,b)+0.05)/(Math.min(a,b)+0.05);
    assert.ok(ratio>=4.5,`${fg} / ${bg}: ${ratio}`);
  }
});

test("transcript source kinds require bounded, distinct native IDs and supported source kinds",()=>{
  assert.deepEqual([...transcriptTrackKinds({tracks:[{id:"not-a-prefix",kind:"mic"},{id:"system-id",kind:"system"}]})],[["not-a-prefix","mic"],["system-id","system"]]);
  assert.equal(transcriptTrackKinds({tracks:[{id:"source",kind:"imported"}]}).get("source"),"imported");
  for(const value of [null,{}, {tracks:[]},{tracks:new Array(1025).fill({id:"x",kind:"mic"})},
    {tracks:[{id:"x",kind:"mic"},{id:"x",kind:"system"}]},{tracks:[{id:"mic",kind:"processed"}]},
    {tracks:[{id:"",kind:"mic"}]},{tracks:[{id:"x".repeat(257),kind:"system"}]},{tracks:[null]}])assert.throws(()=>transcriptTrackKinds(value));
});
