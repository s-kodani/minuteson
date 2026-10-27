import test from "node:test";
import assert from "node:assert/strict";
import {archivedDiagnosticMessage} from "../src/input-diagnostic-copy.ts";
import {InputJournalNotices,journalMessage} from "../src/input-journal-state.ts";
import type {ArchivedInputView} from "../src/archived-input-controller.ts";
const base:ArchivedInputView={meetingId:"m",sessionId:"s",title:"会議",notices:[],reception:{microphone:"none",system:"none"},loss:{microphone:0,system:0},historyIssue:null,unavailable:false,deliveryUnavailable:false,historySaveIssue:null,checked:true,historySaveConfirmed:true};
test("read-only diagnostic explains why it appears without asserting audio loss",()=>{
  const message=archivedDiagnosticMessage({...base,unavailable:true});
  assert.match(message,/読み取れなかったため表示/);
  assert.match(message,/自動で再確認/);
  assert.doesNotMatch(message,/欠損範囲|保存された音声で/);
  assert.match(message,/保存失敗や音声の欠落が分かったわけではありません/);
  assert.equal(archivedDiagnosticMessage(base),"");
  assert.match(archivedDiagnosticMessage({...base,historySaveIssue:"failed"}),/保存処理でエラー/);
});
test("five-second warning explains the delay and next step without a save-failure claim",()=>{
  const n=new InputJournalNotices();const v={state:"unknown" as const,problem:false};
  n.read(v,"recording",0);assert.equal(n.read(v,"recording",4999),null);
  const warning=n.read(v,"recording",5000)!;
  assert.match(warning.message,/5秒以上/);assert.match(warning.message,/自動で再確認/);
  assert.match(warning.message,/録音ファイルの保存失敗を意味するものではありません/);
  assert.match(journalMessage({state:"failed",problem:true}),/保存処理でエラーが発生したため/);
});
