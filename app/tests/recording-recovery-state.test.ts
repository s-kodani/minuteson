import assert from "node:assert/strict";
import test from "node:test";
import {acceptRecovery, recoveryProgress, recoveryMeetingState, type RecoverySnapshot} from "../src/recording-recovery-state.ts";
import {meetingStatusLabel} from "../src/meeting-status.ts";
import {ProgressRegistry, orderedProgress} from "../src/operation-progress-state.ts";

const running: RecoverySnapshot = {runId:"run",revision:3,phase:"running",error:null,items:[
  {sessionId:"session-a",meetingId:"meeting-a",meetingTitle:"回復する会議",phase:"running",startedAtMs:100,error:null},
  {sessionId:"session-b",meetingId:"meeting-b",meetingTitle:"待つ会議",phase:"queued",startedAtMs:null,error:null},
]};
test("recovery reload is authoritative, stale replies cannot revive ended work", () => {
  const done: RecoverySnapshot = {...running,revision:5,phase:"completed",items:running.items.map(i=>({...i,phase:"succeeded"}))};
  assert.equal(acceptRecovery(null,running),running);
  assert.equal(acceptRecovery(done,running),done);
  assert.equal(recoveryProgress(done).length,0);
  for(const patch of [{revision:-1},{phase:"unknown"},{items:[running.items[0],running.items[0]]},{phase:"completed"},{items:[{...running.items[0],startedAtMs:null}]}]) assert.throws(()=>acceptRecovery(null,{...running,...patch}));
});
test("recovery and unrelated analysis coexist, running recovery is visible first without a percentage", () => {
  const items=recoveryProgress(running);
  assert.equal(items.length,2); assert.equal(items[0].total,null);assert.equal(items[0].startedAtMs,100);
  assert.equal(items[1].startedAtMs,undefined);assert.equal(items[1].state,"queued");
  const analysis={...items[0],id:"analysis",kind:"transcription" as const};
  assert.equal(orderedProgress([analysis,...items])[0].kind,"recording_recovery");
  assert.equal(recoveryProgress({...running,phase:"scanning",items:[]})[0].total,null);
});
test("empty jobs require successful reads from both recovery and analysis; a failed read retains work", () => {
  const registry=new ProgressRegistry();
  registry.setReadStatus(null,"recovery");registry.setReadStatus("");
  assert.equal(registry.getReadSnapshot().state,"loading");
  registry.set("recovery",recoveryProgress(running));registry.setReadStatus("offline","recovery");
  assert.equal(registry.getReadSnapshot().state,"error");assert.equal(registry.getSnapshot().length,2);
  registry.setReadStatus("","recovery");assert.equal(registry.getReadSnapshot().state,"ready");
});
test("meeting labels distinguish recovery, waiting, failure and normal saved audio", () => {
  const bridge={kind:"ready" as const,snapshot:{revision:0,phase:"idle" as const,elapsedMs:0,microphoneConnected:false,systemAudioConnected:false,meetingId:null,sessionId:null,errorMessage:null}};
  const meeting={id:"meeting-a",lifecycle:"recording"};
  assert.equal(meetingStatusLabel(meeting,bridge,running),"録音を回復中");
  assert.equal(meetingStatusLabel({...meeting,id:"meeting-b"},bridge,running),"録音の回復待ち");
  const failed={...running,items:[{...running.items[0],phase:"failed" as const,error:"invalid audio"}]};
  assert.equal(meetingStatusLabel(meeting,bridge,failed),"録音未完了・要確認");
  assert.equal(meetingStatusLabel({...meeting,lifecycle:"recorded"},bridge,failed),"録音済み");
  assert.equal(recoveryMeetingState(running,"another"),undefined);
  assert.equal(meetingStatusLabel({...meeting,lifecycle:"recorded"},bridge,{...running,phase:"completed",items:[]}),"録音済み");
});
