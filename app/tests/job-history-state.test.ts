import test from "node:test";
import assert from "node:assert/strict";
import {checkedHistoryPage,historyElapsed,mergeHistory,type JobHistoryEntry} from "../src/job-history-state.ts";
import {voiceModelName,voicePersonModels} from "../src/voice-state.ts";
const row:JobHistoryEntry={id:"job",kind:"transcription",target:"会議",meetingId:"meeting",state:"succeeded",acceptedAt:1000,finishedAt:62000,observedAt:62000,detail:null};
test("history readbacks validate states, identity and page bounds",()=>{
  const page={items:[row],nextCursor:null,storageError:null};assert.equal(checkedHistoryPage(page),page);
  for(const items of [[row,row],[{...row,state:"imagined"}],[{...row,finishedAt:-1}],[{...row,kind:"unknown"}]])assert.throws(()=>checkedHistoryPage({...page,items}));
  assert.equal(mergeHistory([row],[{...row,target:"更新後"}]).length,1);
});
test("elapsed history time never invents an end or negative duration",()=>{
  assert.equal(historyElapsed(row),"1分1秒");assert.equal(historyElapsed({...row,finishedAt:null}),"—");assert.equal(historyElapsed({...row,finishedAt:0}),"—");
});
test("voice names describe registered embeddings, deduplicate names and preserve unknown identity",()=>{
  const known={modelName:"3D-Speaker ERes2Net base",modelHash:"a".repeat(64)};
  const unknown={modelName:null,modelHash:"b".repeat(64)};
  assert.equal(voiceModelName(known),known.modelName);assert.match(voiceModelName(unknown),/モデル名不明.*bbbbbbbb/);
  assert.equal(voicePersonModels({samples:[known,known] as never}),known.modelName);
  assert.equal(voicePersonModels({samples:[]}),"登録モデルなし");
  assert.match(voicePersonModels({samples:[known,unknown] as never}),/3D-Speaker.*モデル名不明/);
});
