import test from "node:test";
import assert from "node:assert/strict";
import { assertMinutesReview, assertMinutesVersion, minutesConfigured, minutesExportRequest, exportLabel, evidenceRequest, assertEvidenceSource, assertMinutesTranscriptPage, minutesTranscriptAudioRequest, itemEvidencePage, type MinutesContent, type MinutesTranscriptPage, type Evidence, type MinutesEvidenceSource, type MinutesView, type MinutesEngine, type MinutesVersions } from "../src/minutes-state.ts";

const engine: MinutesEngine = { executablePath: "/engine/llama-completion", modelPath: "/model.gguf", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 };
const state: MinutesVersions = { meetingLockVersion: 1, currentId: "current", previousId: "previous", draft: { id: "draft", lockVersion: 3 }, versions: [
  { id: "candidate", revisionNo: 3, contentHash: "hash", publicationState: "candidate", inputAnalysisId: "fixed", inputUpdated: false, itemCount: 2, createdAt: 0, profile: {} },
  { id: "previous", revisionNo: 1, contentHash: "previous-hash", publicationState: "committed", inputAnalysisId: "old-fixed", inputUpdated: true, itemCount: 1, createdAt: 0, profile: {} },
] };
test("item evidence groups show every own link in bounded pages and locate the clicked reference without mixing items", () => {
  const own=Array.from({length:23},(_,i):Evidence=>({id:`e${i}`,minutesItemId:"item",analysisRevisionId:"fixed",utteranceId:`u${i}`,quoteText:`引用${i}`,startUs:i*10,endUs:i*10+5,sourceType:"engine",verificationState:"unverified"}));
  const other={...own[0],id:"other",minutesItemId:"other-item"};
  const item={id:"item",kind:"decision" as const,body:"決定事項",assigneeName:null,assigneeSpeakerId:null,dueDate:null,dueText:null,sourceType:"engine" as const};
  const content:MinutesContent={schemaVersion:1,blocks:[],items:[item,{...item,id:"other-item"}],evidence:[...own.slice(0,4),other,...own.slice(4)]};
  const located=itemEvidencePage(content,"e20")!;
  assert.equal(located.page,2); assert.equal(located.pages,3);assert.equal(located.total,23);assert.equal(located.first,20);assert.deepEqual(located.evidence.map(e=>e.id),["e20","e21","e22"]);
  const all=[0,1,2].flatMap(p=>itemEvidencePage(content,"e20",p)!.evidence);
  assert.deepEqual(all,own);assert.equal(new Set(all.map(e=>e.id)).size,23);
  assert.equal(itemEvidencePage(content,"other")!.total,1);
  assert.equal(itemEvidencePage(content,"missing"),null);assert.equal(itemEvidencePage({...content,items:[]},"e20"),null);
  assert.equal(itemEvidencePage(content,"e20",99)!.page,2);assert.equal(itemEvidencePage(content,"e20",-1)!.page,0);assert.equal(itemEvidencePage(content,"e20",NaN)!.page,0);
  assert.equal(content.evidence.length,24);
});
test("minutes engine requires integral bounded parameters and context for the fixed input", () => {
  assert.ok(minutesConfigured(engine));
  for (const patch of [{ modelPath: " " }, { executablePath: "" }, { threads: 0 }, { threads: 1.5 }, { contextTokens: 32769 }, { predictionTokens: 8192 }, { gpuLayers: 257 }, { seed: -1 }, { promptBudget: 6000 }, { promptBudget: 2047 }]) assert.equal(minutesConfigured({ ...engine, ...patch }), false);
});
test("minutes review rejects replacement of the reviewed candidate, head, draft or input status", () => {
  const reviewed = state.versions[0];
  assert.doesNotThrow(() => assertMinutesReview(state, state, reviewed, false));
  assert.doesNotThrow(() => assertMinutesVersion(state, state.versions[1], true));
  assert.throws(() => assertMinutesVersion(state, reviewed, true));
  assert.throws(() => assertMinutesVersion(state, { ...reviewed, contentHash: "other" }, false));
  for (const patch of [{ meetingLockVersion: 2 }, { currentId: "new" }, { previousId: null }, { draft: null }, { draft: { id: "new-draft", lockVersion: 3 } }, { draft: { id: "draft", lockVersion: 4 } }, { versions: [{ ...reviewed, inputUpdated: true }] }]) assert.throws(() => assertMinutesReview({ ...state, ...patch }, state, reviewed, false));
});
test("export requests freeze the saved current head and draft stamp, never an unadopted candidate", () => {
  const committed = { ...state.versions[1], id: "current", contentHash: "current-hash" };
  const current = { ...state, versions: [...state.versions, committed] };
  assert.deepEqual(minutesExportRequest(current, "meeting", "operation"), { operationId: "operation", meetingId: "meeting", expectedMinutesId: "current", expectedContentHash: "current-hash", expectedMeetingLockVersion: 1, expectedDraft: { id: "draft", lockVersion: 3 } });
  assert.throws(() => minutesExportRequest(state, "meeting", "operation"));
  assert.throws(() => minutesExportRequest({ ...state, currentId: "candidate" }, "meeting", "operation"));
  assert.match(exportLabel("succeeded"), /完了/); assert.match(exportLabel("failed"), /失敗/);
  assert.match(exportLabel("writing"), /未確定/); assert.match(exportLabel("interrupted"), /未確定/);
});
test("evidence requests stay bound to the minutes input and reject redirected source responses", () => {
  const e: Evidence = {id:"ref",minutesItemId:"item",analysisRevisionId:"fixed",utteranceId:"u",quoteText:"引用",startUs:10,endUs:20,sourceType:"engine",verificationState:"unverified"};
  const doc = {schemaVersion:1,blocks:[],items:[],evidence:[e]};
  const view: MinutesView = {id:"minutes",meetingId:"meeting",inputAnalysisId:"fixed",inputContentHash:"input-hash",contentHash:"root-hash",content:doc,generatedContent:doc};
  const r=evidenceRequest(view,e);
  assert.equal(r.minutesId,"minutes"); assert.equal(r.expectedContentHash,"root-hash");
  assert.notEqual(r.expectedEvidence,e);
  assert.throws(()=>evidenceRequest({...view,inputAnalysisId:"current"},e));
  const source: MinutesEvidenceSource = {evidence:e,analysisId:"fixed",analysisContentHash:"input-hash",speakers:[],utterances:[{id:"u",trackId:"track",startUs:0,endUs:30,recognizedText:"原文",effectiveText:"前後引用文脈",attributionState:"unknown",speakerIds:[]}],firstSequence:1,totalUtterances:1};
  assert.doesNotThrow(()=>assertEvidenceSource(r,source));
  assert.throws(()=>assertEvidenceSource(r,{...source,analysisContentHash:"changed"}));
  for (const patch of [{analysisId:"current"},{evidence:{...e,quoteText:"差替え"}},{evidence:{...e,endUs:21}},{utterances:[]},{utterances:[{...source.utterances[0],effectiveText:"別の発言"}]},{utterances:[{...source.utterances[0],startUs:11}]},{utterances:Array(6).fill(source.utterances[0])}]) assert.throws(()=>assertEvidenceSource(r,{...source,...patch}));
});

test("fixed transcript pages bind the referenced location and reject wrong, incomplete, duplicated or redirected pages", () => {
  const e: Evidence = {id:"ref",minutesItemId:"item",analysisRevisionId:"fixed",utteranceId:"u65",quoteText:"引用",startUs:650,endUs:660,sourceType:"engine",verificationState:"unverified"};
  const r = {meetingId:"m",minutesId:"minutes",expectedContentHash:"root",expectedInputContentHash:"input",expectedEvidence:e};
  const page = (n:number): MinutesTranscriptPage => ({ evidence:e,analysisId:"fixed",analysisContentHash:"input",speakers:[],firstSequence:n*50+1,totalUtterances:125,page:n,pageSize:50,targetSequence:66,utterances:Array.from({length:Math.min(50,125-n*50)},(_,i)=>({id:`u${n*50+i}`,trackId:"track",startUs:(n*50+i)*10,endUs:(n*50+i+1)*10,recognizedText:"原文",effectiveText:`発言${n*50+i}引用`,attributionState:"unknown",speakerIds:[]})) });
  assert.doesNotThrow(()=>assertMinutesTranscriptPage({reference:r,page:null},page(1)));
  for (const n of [0,1,2]) assert.doesNotThrow(()=>assertMinutesTranscriptPage({reference:r,page:n},page(n),page(1)));
  assert.throws(()=>assertMinutesTranscriptPage({reference:r,page:null},page(0)));
  assert.throws(()=>assertMinutesTranscriptPage({reference:r,page:2},page(1)));
  assert.throws(()=>assertMinutesTranscriptPage({reference:r,page:-1},page(1)));
  const p=page(1);
  for(const patch of [{analysisId:"current"},{analysisContentHash:"changed"},{pageSize:51},{targetSequence:126},{firstSequence:50},{totalUtterances:NaN},{utterances:p.utterances.slice(1)},{utterances:[p.utterances[1],...p.utterances.slice(1)]},{utterances:p.utterances.map((u,i)=>i===15?{...u,effectiveText:"changed"}:u)}])assert.throws(()=>assertMinutesTranscriptPage({reference:r,page:1},{...p,...patch}));
  for(const patch of [{startUs:-1},{endUs:0},{startUs:NaN},{endUs:Number.MAX_SAFE_INTEGER+1},{trackId:""}]) assert.throws(()=>assertMinutesTranscriptPage({reference:r,page:1},{...p,utterances:[{...p.utterances[0],...patch},...p.utterances.slice(1)]}));
  assert.throws(()=>assertMinutesTranscriptPage({reference:r,page:1},p,{targetSequence:66,totalUtterances:126}));
  assert.throws(()=>assertMinutesTranscriptPage({reference:r,page:1},p,{targetSequence:67,totalUtterances:125}));
});

test("fixed workspace audio requires a checked page and its own frozen utterance interval", () => {
  const e: Evidence = {id:"ref",minutesItemId:"item",analysisRevisionId:"fixed",utteranceId:"u",quoteText:"引用",startUs:10,endUs:20,sourceType:"engine",verificationState:"unverified"};
  const content={schemaVersion:1,blocks:[],items:[],evidence:[e]};
  const view: MinutesView={id:"minutes",meetingId:"m",inputAnalysisId:"fixed",inputContentHash:"input",contentHash:"root",content,generatedContent:content};
  const page: MinutesTranscriptPage={evidence:e,analysisId:"fixed",analysisContentHash:"input",speakers:[],firstSequence:1,totalUtterances:2,page:0,pageSize:50,targetSequence:1,utterances:[{id:"u",trackId:"track",startUs:0,endUs:30,recognizedText:"原文",effectiveText:"固定引用",attributionState:"unknown",speakerIds:[]},{id:"other-fixed-u",trackId:"track",startUs:30,endUs:60,recognizedText:"原文",effectiveText:"前後",attributionState:"unknown",speakerIds:[]}]};
  const r=minutesTranscriptAudioRequest(view,e,page,"other-fixed-u");
  assert.equal(r.reference.minutesId,"minutes");assert.equal(r.reference.expectedInputContentHash,"input");assert.equal(r.utteranceId,"other-fixed-u");assert.equal(r.expectedStartUs,30);assert.equal(r.expectedEndUs,60);
  assert.throws(()=>minutesTranscriptAudioRequest(view,e,page,"current-id"));
  assert.throws(()=>minutesTranscriptAudioRequest(view,e,{...page,analysisId:"current"},"u"));
  assert.notEqual(r.reference.expectedEvidence,e);
});
