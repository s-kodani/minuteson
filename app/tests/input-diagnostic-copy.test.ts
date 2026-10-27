import test from "node:test";
import assert from "node:assert/strict";
import {archivedDiagnosticMessage} from "../src/input-diagnostic-copy.ts";
import {InputJournalNotices,journalMessage} from "../src/input-journal-state.ts";
import type {ArchivedInputView} from "../src/archived-input-controller.ts";
const base:ArchivedInputView={meetingId:"m",sessionId:"s",title:"会議",notices:[],reception:{microphone:"none",system:"none"},loss:{microphone:0,system:0},historyIssue:null,unavailable:false,deliveryUnavailable:false,historySaveIssue:null,checked:true,historySaveConfirmed:true};
test("automatic diagnostics stay silent, including mixed read and delivery delays",()=>{
  for(const historySaveIssue of [null,"unconfirmed","unavailable"] as const){
    assert.equal(archivedDiagnosticMessage({...base,unavailable:true,deliveryUnavailable:true,historySaveIssue,reception:{microphone:"unconfirmed",system:"unconfirmed"}}),"");
  }
  assert.equal(archivedDiagnosticMessage(base),"");
});

test("confirmed history failures and losses retain an actionable audio-review notice",()=>{
  for(const patch of [{historySaveIssue:"failed" as const},{historySaveIssue:"invalid" as const},{historyIssue:"missing" as const},{historyIssue:"invalid" as const},{loss:{microphone:2,system:0}}]){
    const message=archivedDiagnosticMessage({...base,...patch,unavailable:true});
    assert.match(message,/会議を開き、音声を再生して内容を確認/);
    assert.doesNotMatch(message,/解消済み|自動で再確認/);
  }
});

test("long journal delays never warn; confirmed failures retain their next action",()=>{
  const n=new InputJournalNotices();const v={state:"unknown" as const,problem:false};
  for(const at of [0,4999,5000,60000])assert.equal(n.read(v,"recording",at),null);
  assert.match(journalMessage({state:"failed",problem:true}),/保存処理でエラーが発生したため/);
});
