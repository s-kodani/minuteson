import assert from "node:assert/strict";
import test from "node:test";
import {acceptRecovery, recoveryProgress, recordingSaveJobs, recoveryMeetingState, type RecoverySnapshot} from "../src/recording-recovery-state.ts";
import {meetingStatusLabel} from "../src/meeting-status.ts";
import {ProgressRegistry, orderedProgress} from "../src/operation-progress-state.ts";

const running: RecoverySnapshot = {runId:"run",revision:3,origin:"startup",targetSessionId:null,phase:"running",error:null,items:[
  {sessionId:"session-a",meetingId:"meeting-a",meetingTitle:"回復する会議",phase:"running",startedAtMs:100,error:null},
  {sessionId:"session-b",meetingId:"meeting-b",meetingTitle:"待つ会議",phase:"queued",startedAtMs:null,error:null},
]};
test("recovery reload is authoritative, stale replies cannot revive ended work", () => {
  const done: RecoverySnapshot = {...running,revision:5,phase:"completed",items:running.items.map(i=>({...i,phase:"succeeded"}))};
  assert.equal(acceptRecovery(null,running),running);
  assert.equal(acceptRecovery(done,running),done);
  assert.equal(recoveryProgress(done).length,0);
  for(const patch of [{revision:-1},{origin:"unknown"},{origin:"retry"},{targetSessionId:42},{phase:"unknown"},{items:[running.items[0],running.items[0]]},{phase:"completed"},{items:[{...running.items[0],startedAtMs:null}]}]) assert.throws(()=>acceptRecovery(null,{...running,...patch}));
});

test("one recording save row survives stop, inventory scan, inner progress and stale outer saving", () => {
  const outer={revision:4,phase:"stopping" as const,sessionId:"session-a",meetingId:"meeting-a",elapsedMs:1000,microphoneConnected:false,systemAudioConnected:false,errorMessage:null};
  const scan={...running,origin:"stop" as const,targetSessionId:"session-a",phase:"scanning" as const,items:[]};
  const first=recordingSaveJobs(outer,null,"会議");
  const scanning=recordingSaveJobs(outer,scan,"会議");
  assert.equal(first.length,1);assert.equal(scanning.length,1);
  assert.equal(first[0].id,scanning[0].id);assert.equal(scanning[0].target,"会議");
  const inner={...scan,phase:"running" as const,items:[{...running.items[0],progress:{stage:"verifying_audio",completed:2,total:4,unit:"files" as const}}]};
  const saving=recordingSaveJobs({...outer,phase:"saving"},inner,"会議");
  assert.equal(saving.length,1);assert.equal(saving[0].id,first[0].id);assert.equal(saving[0].label,"生成音声を検証中");
  const done={...inner,phase:"completed" as const,revision:5,items:inner.items.map(i=>({...i,phase:"succeeded" as const}))};
  assert.deepEqual(recordingSaveJobs({...outer,phase:"saving"},done),[]);
  const retry={...done,origin:"retry" as const,revision:6,phase:"scanning" as const};
  assert.equal(recordingSaveJobs(outer,retry).length,1);
  assert.match(recordingSaveJobs(outer,retry)[0].detail!,/再試行/);
});
test("recovery and unrelated analysis coexist, running recovery is visible first without a percentage", () => {
  const items=recoveryProgress(running);
  assert.equal(items.length,2); assert.equal(items[0].total,null);assert.equal(items[0].startedAtMs,100);
  assert.equal(items[1].startedAtMs,undefined);assert.equal(items[1].state,"queued");
  const analysis={...items[0],id:"analysis",kind:"transcription" as const};
  assert.equal(orderedProgress([analysis,...items])[0].kind,"recording_save");
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
  assert.equal(meetingStatusLabel(meeting,bridge,running),"録音を保存中");
  assert.equal(meetingStatusLabel({...meeting,id:"meeting-b"},bridge,running),"録音の保存待ち");
  const failed={...running,items:[{...running.items[0],phase:"failed" as const,error:"invalid audio"}]};
  assert.equal(meetingStatusLabel(meeting,bridge,failed),"音声保存エラー");
  assert.equal(meetingStatusLabel({...meeting,lifecycle:"recorded"},bridge,failed),"音声保存エラー");
  assert.equal(meetingStatusLabel(meeting,bridge,{...failed,items:failed.items.map(item=>({...item,noAudio:true}))}),"録音失敗");
  assert.throws(()=>acceptRecovery(null,{...failed,phase:"completed",items:failed.items.map(item=>({...item,noAudio:"unknown"}))}));
  assert.equal(recoveryMeetingState(running,"another"),undefined);
  assert.equal(meetingStatusLabel({...meeting,lifecycle:"recorded"},bridge,{...running,phase:"completed",items:[]}),"録音済み");
});
