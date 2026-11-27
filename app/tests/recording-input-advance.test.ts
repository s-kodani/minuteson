import test from "node:test";
import assert from "node:assert/strict";
import {RecoverableRead,sharedClock} from "../src/recoverable-read.ts";
import {advanceSharedRecordingClock} from "../src/recording-input-advance.ts";
import {RecordingInputSession,checkedInputStatus} from "../src/recording-input-state.ts";
import {InputJournalSession,InputJournalFailures,checkedJournalReport} from "../src/input-journal-state.ts";

const id="735027d4-a03e-43d3-830a-21e767dfaf88";
const binding=()=>({state:"connected" as const,sourceEpoch:1,interruptions:0,reason:null});
const status=(revision:number)=>({schemaVersion:4 as const,ownerId:id,revision,reportAgeMs:0,microphone:binding(),system:binding(),notices:[],noticeLoss:{microphone:0,system:0}});
const journal=(revision:number,state:"confirmed"|"failed"="confirmed")=>({schemaVersion:1 as const,ownerId:id,revision,reportAgeMs:0,persistence:{requestedRevision:revision,committedRevision:revision,state,closed:false}});

class JournalAdapter{
  private session:InputJournalSession;
  constructor(owner:string){this.session=new InputJournalSession(owner);}
  reset(owner:string|null){if(owner)this.session=new InputJournalSession(owner);}
  accept(value:unknown,at:number){this.session.accept(value,at);return true;}
  read(at:number){return this.session.read(at);}
}

test("status-only reversal advances shared clock for both polls",async()=>{
  let mono=1000,wall=10000,statusRev=1,journalRev=1;
  const clock=sharedClock(mono,wall);
  const statusSession=new RecordingInputSession();statusSession.reset(id);
  const journalSession=new JournalAdapter(id);
  const failures=new InputJournalFailures();
  const monoFn=()=>mono,wallFn=()=>wall;
  const statusPoll=new RecoverableRead(id,statusSession,clock,{mono:monoFn,wall:wallFn,
    revision:v=>{try{return {revision:checkedInputStatus(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>Promise.resolve(status(statusRev)),
  });
  const journalPoll=new RecoverableRead(id,journalSession,clock,{mono:monoFn,wall:wallFn,
    revision:v=>{try{return {revision:checkedJournalReport(v,id).revision,reportAgeMs:0};}catch{return null;}},
    invoke:()=>Promise.resolve(journal(journalRev)),
  });
  statusPoll.poll();journalPoll.poll();await Promise.resolve();
  assert.equal(statusPoll.read().microphone.state,"connected");
  assert.equal(journalPoll.read().state,"confirmed");
  statusRev=2;journalRev=2;journal(journalRev,"failed");
  const failedView={state:"failed" as const,requestedRevision:2,committedRevision:1,problem:false};
  journalSession.accept(journal(2,"failed"),clock.read(mono,wall));
  assert.equal(failures.read(failedView,"recording",clock.read(mono,wall)),"failed");
  mono=900;wall=9000;
  advanceSharedRecordingClock(clock,statusPoll,journalPoll,monoFn,wallFn);
  assert.equal(statusPoll.read().microphone.state,"unknown");
  assert.equal(journalPoll.read().state,"unknown");
  statusPoll.poll();journalPoll.poll();await Promise.resolve();
  statusRev=2;journalRev=2;
  statusPoll.poll();journalPoll.poll();await Promise.resolve();
  statusRev=3;journalRev=3;
  statusPoll.poll();journalPoll.poll();await Promise.resolve();
  assert.equal(statusPoll.read().microphone.state,"connected");
  assert.equal(journalPoll.read().state,"confirmed");
  assert.equal(failures.read(journalPoll.read(),"recording",clock.read(mono,wall)),"failed");
  statusPoll.dispose();journalPoll.dispose();
});
