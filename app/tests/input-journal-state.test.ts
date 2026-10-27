import test from "node:test";
import assert from "node:assert/strict";
import {InputJournalSession,InputJournalNotices,checkedJournalReport,journalMessage,type JournalPersistence,type JournalView} from "../src/input-journal-state.ts";
import {ToastController} from "../src/toast-state.ts";
import {ArchivedInputController} from "../src/archived-input-controller.ts";
import {checkedArchivedInputPage} from "../src/archived-input-state.ts";
const owner="73500000-0000-4000-8000-000000000001",meeting="63500000-0000-4000-8000-000000000001";
const report=(revision=1,state:JournalPersistence["state"]="pending",requestedRevision=999,committedRevision=998)=>({schemaVersion:1,ownerId:owner,revision,reportAgeMs:0,persistence:{state,requestedRevision,committedRevision,closed:false}});
test("journal diagnostic DTO is closed, bounded and independent of heartbeat revision",()=>{
  assert.equal(checkedJournalReport(report(),owner).persistence.requestedRevision,999);
  for(const bad of [{...report(),ownerId:meeting},{...report(),audioSaved:true},{...report(),revision:0},{...report(),reportAgeMs:NaN},
    report(1,"confirmed"),report(1,"pending",1,2),{...report(),persistence:{...report().persistence,state:"saved"}},
    {...report(),persistence:{...report().persistence,closed:1}}])assert.throws(()=>checkedJournalReport(bad,owner));
});
test("late request latency and repeated same heartbeat expire without a success claim",()=>{
  const s=new InputJournalSession(owner);s.accept(report(),10);
  assert.equal(s.read(1010).state,"pending");assert.equal(s.read(1011).state,"stale");
  s.accept(report(),1012);assert.equal(s.read(1012).state,"stale");
  s.accept(report(2),1013);assert.equal(s.read(1013).state,"pending");
});
test("failure survives absent/failed/stale reads and only fresh confirmation resolves it",()=>{
  const s=new InputJournalSession(owner);s.accept(report(1,"failed"),0);assert.equal(s.read(0).problem,true);
  s.fail();assert.deepEqual(s.read(1),{state:"unknown",problem:true});
  s.accept(report(2),2);assert.equal(s.read(2).problem,true);
  assert.match(journalMessage(s.read(2)),/以前の書込み失敗/u);
  assert.equal(s.read(1003).problem,true);
  s.accept(report(3,"confirmed",999,999),1004);assert.deepEqual(s.read(1004),{state:"confirmed",problem:false});
});
const view=(state:JournalView["state"],problem=false):JournalView=>({state,problem});
test("preparation and one-to-three-second diagnostic delays do not notify; five seconds does",()=>{
  const notices=new InputJournalNotices();
  assert.equal(notices.read(view("unknown"),"preparing",0),null);
  assert.equal(notices.read(view("unknown"),"preparing",10000),null);
  for(const at of [10000,11000,12000,13000,14999])assert.equal(notices.read(view("stale"),"recording",at),null);
  const warning=notices.read(view("stale"),"recording",15000)!;
  assert.equal(warning.level,"warning");
  assert.equal(notices.read(view("pending"),"recording",15500),warning);
  assert.equal(notices.read(view("unknown"),"recording",16000),warning);
});
test("pending time starts with the pending result, even late in a recording",()=>{
  const n=new InputJournalNotices();n.read(view("confirmed"),"recording",0);
  assert.equal(n.read(view("pending"),"recording",60000),null);
  assert.equal(n.read(view("pending"),"recording",64999),null);
  assert.equal(n.read(view("pending"),"recording",65000)?.level,"warning");
});
test("short recoveries keep the same dismissed occurrence; two seconds of fresh reports resolve it",()=>{
  const n=new InputJournalNotices(),toast=new ToastController({now:()=>0,start:()=>0,stop:()=>{}});
  n.read(view("stale"),"recording",0);
  const warning=n.read(view("stale"),"recording",5000)!;
  toast.show({key:owner,...warning});toast.dismiss(owner,toast.getSnapshot()[0].id);
  for(const [at,state] of [[5500,"confirmed"],[6000,"unknown"],[6500,"pending"],[7000,"stale"],[7500,"confirmed"],[8000,"confirmed"],[8500,"confirmed"],[9000,"confirmed"]] as const){
    const next=n.read(view(state),"recording",at)!;
    assert.equal(next,warning);toast.show({key:owner,...next});assert.equal(toast.getSnapshot().length,0);
  }
  assert.equal(n.read(view("confirmed"),"recording",9500),null);toast.resolve(owner);
  n.read(view("unknown"),"recording",10000);
  const next=n.read(view("unknown"),"recording",15000)!;
  assert.notEqual(next.eventId,warning.eventId);toast.show({key:owner,...next});assert.equal(toast.getSnapshot().length,1);
});
test("stale confirmations and missing observations never establish stable recovery",()=>{
  const n=new InputJournalNotices(),s=new InputJournalSession(owner);
  n.read(view("unknown"),"recording",0);const warning=n.read(view("unknown"),"recording",5000);
  s.accept(report(1,"confirmed",999,999),5500);
  for(const at of [5500,6000,6500,7000,7500])assert.equal(n.read(s.read(at),"recording",at),warning);
  n.read(view("confirmed"),"recording",8000);
  assert.equal(n.read(view("confirmed"),"recording",12000),warning);
});
test("real failures notify immediately, survive missing reads, escalate invalidity and end with the owner",()=>{
  const n=new InputJournalNotices();
  const failed=n.read(view("failed",true),"recording",0)!;assert.equal(failed.level,"error");
  assert.equal(n.read(view("unknown",true),"recording",500),failed);
  assert.equal(n.read(view("pending",true),"recording",1000),failed);
  const invalid=n.read(view("invalid",true),"recording",1500)!;
  assert.notEqual(invalid.eventId,failed.eventId);assert.match(invalid.message,/整合/u);
  assert.equal(n.read(view("stale",true),"recording",2000),invalid);
  assert.equal(n.read(view("unknown",true),"saved",2500),null);
  assert.equal(new InputJournalNotices().read(view("unknown"),"recording",2500),null);
});
test("immutable heartbeat, committed floors, closed and invalid state cannot regress",()=>{
  const s=new InputJournalSession(owner);s.accept(report(),0);
  for(const bad of [report(1,"failed"),report(2,"pending",998,997),report(0)])assert.throws(()=>s.accept(bad,1));
  s.accept({...report(2,"invalid"),persistence:{...report(2,"invalid").persistence,closed:true}},2);
  assert.throws(()=>s.accept(report(3,"confirmed",999,999),3));
  assert.throws(()=>s.accept(report(3,"invalid"),3));
});
test("clock reversal permanently invalidates; caller cannot mutate accepted evidence",()=>{
  const s=new InputJournalSession(owner),value=report();s.accept(value,10);value.persistence.state="confirmed";
  assert.equal(s.read(11).state,"pending");assert.equal(s.read(9).state,"unknown");assert.throws(()=>s.accept(report(2),12));
});
test("archive page v3 diagnoses final writer failure independently of an absent queue",()=>{
  const row={meetingId:meeting,sessionId:owner,title:"会議",batch:null,issue:null,historySaveIssue:"unconfirmed"};
  const page={schemaVersion:3,recordings:[row],nextCursor:null};
  assert.equal(checkedArchivedInputPage(page,null).recordings[0].historySaveIssue,"unconfirmed");
  const c=new ArchivedInputController(meeting);c.acceptPage(page,null,0);
  assert.equal(c.paint(0).views[0].historySaveIssue,"unconfirmed");assert.deepEqual(c.paint(0).attempts,[]);
  c.failedRead();assert.equal(c.paint(1).views[0].historySaveIssue,"unconfirmed");
  const {historySaveIssue:_,...legacyRow}=row;
  assert.throws(()=>c.acceptPage({schemaVersion:1,recordings:[legacyRow],nextCursor:null},null,2));
  assert.equal(c.paint(2).views[0].historySaveIssue,"unconfirmed");
  assert.throws(()=>checkedArchivedInputPage({...page,schemaVersion:"2"},null));
  for(const bad of [{...page,recordings:[{...row,historySaveIssue:"saved"}]},{...page,schemaVersion:1},
    {...page,recordings:[{meetingId:meeting,sessionId:owner,title:"会議",batch:null,issue:null}]}])assert.throws(()=>checkedArchivedInputPage(bad,null));
});
