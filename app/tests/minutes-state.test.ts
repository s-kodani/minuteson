import test from "node:test";
import assert from "node:assert/strict";
import { assertMinutesReview, assertMinutesVersion, minutesConfigured, minutesExportRequest, exportLabel, evidenceRequest, assertEvidenceSource, type Evidence, type MinutesEvidenceSource, type MinutesView, type MinutesEngine, type MinutesVersions } from "../src/minutes-state.ts";

const engine: MinutesEngine = { executablePath: "/engine/llama-completion", modelPath: "/model.gguf", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 };
const state: MinutesVersions = { meetingLockVersion: 1, currentId: "current", previousId: "previous", draft: { id: "draft", lockVersion: 3 }, versions: [
  { id: "candidate", revisionNo: 3, contentHash: "hash", publicationState: "candidate", inputAnalysisId: "fixed", inputUpdated: false, itemCount: 2, createdAt: 0, profile: {} },
  { id: "previous", revisionNo: 1, contentHash: "previous-hash", publicationState: "committed", inputAnalysisId: "old-fixed", inputUpdated: true, itemCount: 1, createdAt: 0, profile: {} },
] };
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
