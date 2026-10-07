import test from "node:test";
import assert from "node:assert/strict";
import { modelNotice, type ModelOutcome } from "../src/model-notification-state.ts";

test("each model outcome retains its exact operation and bounded result meaning", () => {
  for (const outcome of ["inspected", "placed", "cpuSucceeded", "cpuCancelled", "applied", "applicationStopped", "applicationReleased"] as ModelOutcome[]) {
    const result = modelNotice(outcome, "same-operation", "標準・whisper.cpp");
    assert.equal(result.eventId, "same-operation");
    assert.ok(result.message.length < 60);
    assert.ok(result.details.length > result.message.length);
    assert.throws(() => modelNotice(outcome, ""));
  }
  assert.equal(modelNotice("inspected", "1").level, "info");
  assert.match(modelNotice("inspected", "1").details, /未配置.*起動していません/);
  assert.match(modelNotice("placed", "2").details, /未検証.*準備完了ではありません/);
  assert.match(modelNotice("cpuSucceeded", "3").details, /プロファイルは変更していません.*未評価/);
});
test("a cancellation never claims rollback, success or readiness", () => {
  assert.equal(modelNotice("cpuCancelled", "1").level, "info");
  assert.equal(modelNotice("applicationStopped", "2").level, "warning");
  assert.match(modelNotice("applicationStopped", "2").details, /保存が先に確定.*同じ操作/);
  assert.match(modelNotice("applied", "3", "高精度・llama.cpp").message, /高精度・llama.cpp/);
  assert.match(modelNotice("applied", "3").details, /解析の開始・完了やAI準備完了.*ではありません/);
  assert.equal(modelNotice("applicationReleased", "4").level, "info");
  assert.match(modelNotice("applicationReleased", "4").details, /保存成功・取消・巻き戻し.*ではありません/);
});
