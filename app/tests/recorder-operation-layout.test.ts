import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
test("recording operations and archived input changes use notifications without diagnostic cards",()=>{
  const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");
  const operation=source("recorder-operation-notices.tsx").split("export function RecorderOperationNotifications")[1];
  assert.doesNotMatch(operation,/className="issue-status"|<InfoPopover|<section/);
  assert.match(operation,/operationNames\[e.source\].*e.message/);
  assert.match(operation,/dismiss_recorder_operation_notice/);
  const archive=source("archived-input-notices.tsx");
  assert.doesNotMatch(archive,/<details|<section|type="diagnostic"|終了済み録音の通知履歴：/);
  assert.match(archive,/取得が.*中断していました/);
  assert.match(archive,/onRendered.*actions.current.rendered/);
});
