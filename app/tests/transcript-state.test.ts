import assert from "node:assert/strict";
import test from "node:test";
import { addSpeaker, assignSpeakers, canonicalPayload, mergeSpeaker, projectTranscript, activeSpeakers, type Analysis, type DraftPayload } from "../src/transcript-state.ts";
const base:Analysis={id:"base",meetingId:"m",contentHash:"h",speakers:["a","b","c"].map(id=>({id,label:id,displayName:null})),utterances:[
  {id:"u",startUs:0,endUs:10,recognizedText:"原文",effectiveText:"原文",attributionState:"overlap",speakerIds:["a","b"]},
  {id:"v",startUs:10,endUs:20,recognizedText:"未確認",effectiveText:"未確認",attributionState:"unknown",speakerIds:[]},
]};
const empty=():DraftPayload=>({schema_version:1,speaker_names:[],utterances:[]});
const manual="manual:eb3f25d0-b461-4623-86cc-649a0a3a150a";
test("v1 serialization remains exact and optional empty arrays omitted",()=>{
  assert.equal(JSON.stringify(canonicalPayload({...empty(),manual_speakers:[],speaker_merges:[]})),JSON.stringify(empty()));
});
test("merge keeps text time originals and deduplicates overlap; unknown stays unknown",()=>{
  const old=structuredClone(base), p=mergeSpeaker(base,empty(),"b","a"),view=projectTranscript(base,p);
  assert.deepEqual(base,old); assert.deepEqual(view.utterances[0].speakerIds,["a"]);
  assert.equal(view.utterances[0].effectiveText,"原文"); assert.equal(view.utterances[0].endUs,10);
  assert.equal(view.utterances[1].attributionState,"unknown");
  assert.deepEqual(activeSpeakers(view,p).map(s=>s.id),["a","c"]);
  assert.equal(view.speakers.length,3); // inferred segment labels remain available
});
test("successive merges flatten provenance and reject hidden/cyclic targets",()=>{
  const first=mergeSpeaker(base,empty(),"b","a"),second=mergeSpeaker(base,first,"a","c");
  assert.deepEqual(second.speaker_merges,[{source_id:"a",target_id:"c"},{source_id:"b",target_id:"c"}]);
  assert.deepEqual(projectTranscript(base,second).utterances[0].speakerIds,["c"]);
  assert.throws(()=>mergeSpeaker(base,second,"c","a")); assert.throws(()=>mergeSpeaker(base,empty(),"a","a"));
});
test("new speaker assignment replaces only chosen effective assignments and keeps edits",()=>{
  const p=empty();p.utterances.push({utterance_id:"u",effective_text:"修正済み",speaker_ids:["a","b"]});
  const added=addSpeaker(base,p,manual,"田中","u"),view=projectTranscript(base,added);
  assert.equal(added.schema_version,2); assert.equal(view.speakers[3].displayName,"田中");
  assert.deepEqual(view.utterances[0].speakerIds,[manual]); assert.equal(view.utterances[0].effectiveText,"修正済み");
  assert.deepEqual(view.utterances[1],base.utterances[1]); assert.equal(p.schema_version,1);
  assert.throws(()=>addSpeaker(base,p,manual,"田中","bad"));
  assert.throws(()=>addSpeaker(base,added,manual,"重複"));
});
test("bulk assignments cover all selected pages and preserve every text",()=>{
  const p=addSpeaker(base,empty(),manual,"新話者");
  const changed=assignSpeakers(base,p,["u","v"],[manual]);
  assert.deepEqual(projectTranscript(base,changed).utterances.map(u=>u.speakerIds),[[manual],[manual]]);
  assert.deepEqual(projectTranscript(base,changed).utterances.map(u=>u.effectiveText),["原文","未確認"]);
  assert.deepEqual(projectTranscript(base,assignSpeakers(base,changed,["u"],[])).utterances[0].speakerIds,[]);
  assert.throws(()=>assignSpeakers(base,p,["bad"],[])); assert.throws(()=>assignSpeakers(base,p,["u"],["foreign"]));
});
