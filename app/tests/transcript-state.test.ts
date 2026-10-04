import assert from "node:assert/strict";
import test from "node:test";
import { addSpeaker, assignSpeakers, canonicalPayload, mergeSpeaker, projectTranscript, activeSpeakers, type Analysis, type DraftPayload } from "../src/transcript-state.ts";
import { filterUtterances, splitPosition, splitUtterance, textPieces } from "../src/transcript-state.ts";
const base:Analysis={id:"base",meetingId:"m",contentHash:"h",speakers:["a","b","c"].map(id=>({id,label:id,displayName:null})),utterances:[
  {id:"u",startUs:0,endUs:10,recognizedText:"原文",effectiveText:"原文",attributionState:"overlap",speakerIds:["a","b"]},
  {id:"v",startUs:10,endUs:20,recognizedText:"未確認",effectiveText:"未確認",attributionState:"unknown",speakerIds:[]},
]};
const empty=():DraftPayload=>({schema_version:1,speaker_names:[],utterances:[]});
const manual="manual:eb3f25d0-b461-4623-86cc-649a0a3a150a";
const children:[string,string]=["split:97869d35-27c6-405b-89b2-31b759ef5e66","split:5a81a30b-b6e2-420a-a3b0-94851a97c03d"];
test("projection preserves stored order without splits and uses native track tie breaking with splits",()=>{
  const b=structuredClone(base);b.utterances.reverse();
  assert.deepEqual(projectTranscript(b,empty()).utterances.map(u=>u.id),["v","u"]);
  b.utterances[0].startUs=0;b.utterances[0].trackId="a-track";b.utterances[1].trackId="z-track";
  const p=splitUtterance(b,empty(),"u",5,1,children);
  assert.deepEqual(projectTranscript(b,p).utterances.map(u=>u.id),["v",children[0],children[1]]);
});
test("split preserves every grapheme, times, source original, explicit assignments and immutable base",()=>{
  const b=structuredClone(base);b.utterances[0].effectiveText="はい👨‍👩‍👧‍👦。いいえ。";
  const before=structuredClone(b),p=splitUtterance(b,empty(),"u",5,4,children,[["a"],[]]);
  const view=projectTranscript(b,p),parts=view.utterances.filter(u=>u.sourceUtteranceId==="u");
  assert.deepEqual(b,before);assert.equal(p.schema_version,3);
  assert.equal(parts.map(u=>u.effectiveText).join(""),b.utterances[0].effectiveText);
  assert.equal(parts[0].effectiveText,"はい👨‍👩‍👧‍👦。");
  assert.deepEqual(parts.map(u=>[u.startUs,u.endUs]),[[0,5],[5,10]]);
  assert.deepEqual(parts.map(u=>u.speakerIds),[["a"],[]]);
  assert.ok(parts.every(u=>u.recognizedText==="原文"&&u.recognitionIsSource));
});
test("splitting an edited child flattens the same base operation without losing sibling edits",()=>{
  const p=splitUtterance(base,empty(),"u",5,1,children);
  p.utterances.push({utterance_id:children[1],effective_text:"後半の編集",speaker_ids:["c"]});
  const next=splitUtterance(base,p,children[1],7,2,["split:3aa151cb-3f66-4b09-8a03-e6717d5d6043","split:40417ddf-63e5-4a03-b457-6556e558b39b"]);
  assert.equal(next.utterance_splits?.length,1);assert.equal(next.utterance_splits?.[0].parts.length,3);
  assert.equal(next.utterance_splits?.[0].source_effective_text,"原後半の編集");
  assert.equal(next.utterance_splits?.[0].parts[0].id,children[0]);
  assert.deepEqual(projectTranscript(base,next).utterances.filter(u=>u.sourceUtteranceId).map(u=>u.speakerIds),[["a","b"],["c"],["c"]]);
  assert.equal(mergeSpeaker(base,next,"b","a").schema_version,3);
  assert.equal(addSpeaker(base,next,manual,"新しい話者").schema_version,3);
});
test("split rejects endpoints, duplicate IDs, empty portions and unknown assignments",()=>{
  for(const at of [0,10,5.5,NaN])assert.throws(()=>splitUtterance(base,empty(),"u",at,1,children));
  for(const offset of [0,2,1.5])assert.throws(()=>splitUtterance(base,empty(),"u",5,offset,children));
  assert.throws(()=>splitUtterance(base,empty(),"u",5,1,[children[0],children[0]]));
  assert.throws(()=>splitUtterance(base,empty(),"u",5,1,children,[["foreign"],[]]));
});
test("user cut time is exact microseconds, not inferred word timing",()=>{
  assert.equal(splitPosition(1_000_000,3_000_000,"0.000001"),1_000_001);
  for(const s of ["","1e0","1.0000001","-1","0","2"])assert.throws(()=>splitPosition(1_000_000,3_000_000,s));
  assert.deepEqual(textPieces("か\u3099👨‍👩‍👧‍👦"),["か\u3099","👨‍👩‍👧‍👦"]);
});
test("search and speaker filter combine over all effective split portions",()=>{
  const p=splitUtterance(base,empty(),"u",5,1,children,[["a"],[]]);
  const view=projectTranscript(base,p);
  assert.deepEqual(filterUtterances(view,"unknown","").map(u=>u.id),[children[1],"v"]);
  assert.deepEqual(filterUtterances(view,"a","原").map(u=>u.id),[children[0]]);
  assert.equal(filterUtterances(view,"a","未確認").length,0);
  assert.equal(filterUtterances(view,""," 文 ").length,1);
});
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
