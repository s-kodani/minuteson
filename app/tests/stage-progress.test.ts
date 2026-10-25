import test from "node:test";
import assert from "node:assert/strict";
import {checkedStageProgress, stageDetail, stageLabel} from "../src/stage-progress.ts";
import {deletionProgress} from "../src/operation-progress-state.ts";
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
