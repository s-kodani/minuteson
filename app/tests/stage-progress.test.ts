import test from "node:test";
import assert from "node:assert/strict";
import {checkedStageProgress, stageDetail, stageLabel} from "../src/stage-progress.ts";
import {deletionProgress, operationElapsed, orderedProgress, type OperationProgress} from "../src/operation-progress-state.ts";
import type {DataSnapshot} from "../src/data-operation-state.ts";

test("measured stages distinguish inspection reads from deletion and never invent a percentage", () => {
  const progress = {stage:"verifying_model_delete_scope",completed:1024,total:null,unit:"bytes" as const};
  assert.match(stageDetail(progress)!, /検査の読取り/);
  assert.match(stageLabel(progress)!, /確認中/);
  const s = {kind:"model_deletion",token:"one",busy:true,phase:"choosing",progress} as DataSnapshot;
  const job = deletionProgress(s)!;
  assert.equal(job.id,"data:one"); assert.equal(job.completed,null); assert.equal(job.total,null);
  assert.equal(deletionProgress({...s,busy:false}),null);
  assert.equal(deletionProgress({...s,phase:"cancelling"})!.state,"pausing");
});
test("unknown counts and malformed transient measurements cannot masquerade as measured completion", () => {
  assert.equal(checkedStageProgress(undefined),null);
  const p = {stage:"converting_audio",completed:1,total:2,unit:"files"};
  assert.match(stageDetail(p as never)!, /1 \/ 2/);
  for (const patch of [{completed:-1},{total:0},{completed:Infinity},{completed:1.2},{unit:"percent"},{stage:""}]) {
    assert.throws(()=>checkedStageProgress({...p,...patch}));
  }
});

test("meeting deletion and safe stopping remain the foreground operation with measured counts", () => {
  const snapshot = {kind:"deletion",token:"delete",busy:true,phase:"deleting_files",deletionPreview:{title:"週次会議",meetingId:"meeting"},progress:{stage:"deleting_files",completed:7,total:10,unit:"paths"}} as DataSnapshot;
  const running = deletionProgress(snapshot)!;
  assert.equal(running.target,"週次会議");
  assert.equal(running.meetingId,"meeting");
  assert.equal(running.label,"確認した項目を削除中");
  assert.equal(running.detail,"7 / 10 項目");
  assert.equal(running.completed,null);
  assert.equal(running.total,null);
  assert.equal(operationElapsed(running,Date.now()),undefined);
  const recovery = {id:"recovery",kind:"recording_recovery",state:"running"} as OperationProgress;
  const analysis = {id:"analysis",kind:"transcription",state:"running"} as OperationProgress;
  const stopping = deletionProgress({...snapshot,phase:"cancelling"})!;
  assert.equal(stopping.label,"安全な停止を待機中");
  assert.equal(stopping.state,"pausing");
  for (const deletion of [running,stopping]) assert.deepEqual(orderedProgress([analysis,recovery,deletion]),[deletion,recovery,analysis]);
  assert.equal(deletionProgress({...snapshot,busy:false}),null);
});

test("older snapshots still identify the actual deletion phase without measured progress", () => {
  const s = {kind:"deletion",token:"delete",busy:true,phase:"deleting_database"} as DataSnapshot;
  assert.equal(deletionProgress(s)!.label,"会議の削除結果を登録中");
  assert.equal(deletionProgress({...s,phase:"choosing"})!.label,"会議の削除対象を確認中");
  assert.equal(deletionProgress(s)!.detail,undefined);
});
