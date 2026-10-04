import assert from "node:assert/strict";
import test from "node:test";
import {flowStages,flowContinueLabel,type AnalysisFlow} from "../src/analysis-flow-state.ts";
import type {AnalysisJob} from "../src/analysis-state.ts";
const job=(state:string):AnalysisJob=>({id:"j",kind:"transcription",state,completedUnits:0,totalUnits:2,errorCode:null,outputAnalysisId:null,pauseReason:null,specHash:"h",retryOfJobId:null,attempt:1});
const flow=():AnalysisFlow=>({id:"f",meetingId:"m",stage:"configuration",lockVersion:1,transcriptionJob:null,diarizationJob:null,errorMessage:"models missing"});
test("configuration is not completion and provides an explicit preparation action",()=>{
 const f=flow();assert.equal(flowStages(f)[1].state,"AI準備待ち");assert.equal(flowStages(f)[3].state,"待機中");assert.equal(flowContinueLabel(f),"AI設定を確認して解析を開始");
});
test("partial success leaves text complete while diarization failure remains actionable",()=>{
 const f=flow();f.stage="diarization";f.transcriptionJob=job("succeeded");f.diarizationJob={...job("failed"),kind:"diarization"};
 assert.equal(flowStages(f)[1].state,"完了");assert.match(flowStages(f)[2].state,/失敗/);assert.equal(flowStages(f)[3].state,"待機中");assert.equal(flowContinueLabel(f),null);
});
test("recording pause distinguishes in-progress stop from actual pause; interrupted is not running",()=>{
 const f=flow();f.stage="transcription";f.transcriptionJob={...job("pausing"),pauseReason:"recording"};assert.equal(flowStages(f)[1].state,"録音・取り込み優先で停止処理中");
 f.transcriptionJob.state="paused";assert.equal(flowStages(f)[1].state,"録音・取り込み優先で一時停止");f.transcriptionJob.state="interrupted";assert.match(flowStages(f)[1].state,/再開確認/);
});
test("review is a saved candidate, not an automatically adopted result",()=>{
 const f=flow();f.stage="review";f.transcriptionJob=job("succeeded");f.diarizationJob={...job("succeeded"),kind:"diarization"};assert.equal(flowStages(f)[3].state,"候補を保存済み");assert.equal(flowContinueLabel(f),null);
});
