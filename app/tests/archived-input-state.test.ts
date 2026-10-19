import test from "node:test";
import assert from "node:assert/strict";
import {ArchivedInputSession,checkedArchivedInputPage,checkedArchivedInputBatch,type ArchivedInputBatch,type ArchivedInputRow} from "../src/archived-input-state.ts";
import {ArchivedInputController,type ArchivedAttempt} from "../src/archived-input-controller.ts";
import type {InputNotice} from "../src/recording-input-state.ts";
const id=(i:number)=>`${i.toString(16).padStart(8,"0")}-aaaa-4aaa-8aaa-aaaaaaaaaaaa`;
const owner=id(1),meeting=id(999),recipient=id(1000),op=id(1001);
function notice(sequence=1,generation=1,receiver:string|null=null,rendered=false):InputNotice {
  const count=Math.ceil(sequence/2),kind=sequence%2?"interrupted":"recovered";
  return {id:`${owner}:microphone:${count}:${kind}`,input:"microphone",target:"main",kind,interruptions:count,recipientId:receiver,
    delivery:{sequence,generation,leaseAgeMs:receiver?0:null,receiveId:receiver?op:null,rendered,status:{state:kind==="recovered"?"connected":"disconnected",sourceEpoch:Math.floor(sequence/2)+1,interruptions:count,reason:kind==="recovered"?null:"device_disconnected"}}};
}
const batch=(n:InputNotice[]= [notice()],revision=1):ArchivedInputBatch=>({schemaVersion:1,ownerId:owner,revision,notices:n,noticeLoss:{microphone:0,system:0},historyIssue:null});
const row=(b:ArchivedInputBatch|null=batch(),issue:null|"unavailable"=null):ArchivedInputRow=>({meetingId:meeting,sessionId:owner,title:"会議",batch:b,issue,historySaveIssue:null});
const page=(recordings:ArchivedInputRow[]=[row()],nextCursor:string|null=null)=>({schemaVersion:3,recordings,nextCursor});
const own=(request:ArchivedAttempt,rendered=false):InputNotice=>({...request.attempt.notice,recipientId:recipient,delivery:{...request.attempt.notice.delivery!,leaseAgeMs:0,receiveId:request.attempt.receiveId,rendered}});
const receipt=(r:ArchivedAttempt,rendered=r.attempt.action!=="receive")=>({schemaVersion:2,ownerId:r.attempt.sessionId,noticeId:r.attempt.noticeId,target:"main",recipientId:recipient,delivery:own(r,rendered).delivery});
const create=()=>new ArchivedInputController(recipient);
test("version 3 separates readable imported diagnosis from unavailable delivery without transferred claims",()=>{
  const diagnostic={...row(batch([],1)),issue:"deliveryUnavailable" as const,historySaveIssue:"failed" as const};
  diagnostic.batch!.noticeLoss={microphone:2,system:2};
  const value={schemaVersion:3,recordings:[diagnostic],nextCursor:null};
  assert.deepEqual(checkedArchivedInputPage(value,null),value);
  for(const schemaVersion of [1,2])assert.throws(()=>checkedArchivedInputPage({...value,schemaVersion},null));
  for(const invalid of [
    {...diagnostic,batch:null},
    {...diagnostic,batch:batch()},
    {...diagnostic,batch:batch([notice(1,1,recipient,true)])},
    {...diagnostic,batch:{...diagnostic.batch!,ownerId:meeting}},
    {...diagnostic,issue:"unavailable"},
    {...diagnostic,issue:"confirmed"},
  ])assert.throws(()=>checkedArchivedInputPage({...value,recordings:[invalid]},null));
  const c=create();
  c.acceptPage(value,null,0);
  for(const at of [0,100,2000,6000]){
    const {views,attempts}=c.paint(at);
    assert.deepEqual(attempts,[]);assert.deepEqual(views[0].notices,[]);
    assert.equal(views[0].unavailable,false);assert.equal(views[0].deliveryUnavailable,true);
    assert.equal(views[0].historySaveIssue,"failed");
    assert.deepEqual(views[0].loss,{microphone:2,system:2});
    c.rendered(owner,notice().id,1,at);c.dismiss(owner,"microphone");
  }
  c.pause(true);c.paint(6001);c.pause(false);
  c.acceptPage(value,null,6002);
  assert.deepEqual(c.paint(6002).attempts,[]);
  c.failedRead();
  const state=c.paint(6003);
  assert.equal(state.views[0].unavailable,true);
  assert.equal(state.views[0].deliveryUnavailable,true);
  assert.deepEqual(state.views[0].loss,{microphone:2,system:2});
  assert.equal(state.views[0].historySaveIssue,"failed");
});
test("native version 3 delivery still receives normally and diagnostic-only imports do not suppress it",()=>{
  const native={...row(),historySaveIssue:null};
  const restored={...row(batch([])),sessionId:id(2),batch:{...batch([]),ownerId:id(2),noticeLoss:{microphone:2,system:0}},issue:"deliveryUnavailable" as const,historySaveIssue:"unconfirmed" as const};
  const c=create();c.acceptPage({schemaVersion:3,recordings:[native,restored],nextCursor:null},null,0);
  const state=c.paint(0);
  assert.equal(state.attempts.length,1);assert.equal(state.attempts[0].attempt.sessionId,owner);
  assert.equal(state.views[0].deliveryUnavailable,false);
  assert.equal(state.views[1].deliveryUnavailable,true);
  assert.deepEqual(state.views[1].notices,[]);assert.equal(state.views[1].historySaveIssue,"unconfirmed");
  assert.equal(c.complete(state.attempts[0],receipt(state.attempts[0]),1),true);
});
test("archive DTO is closed and has historical events, never current capture or report age",()=>{
  assert.deepEqual(checkedArchivedInputBatch(batch(),owner),batch());
  for(const extra of ["reportAgeMs","microphone","sealed","currentInputs"]){assert.throws(()=>checkedArchivedInputBatch({...batch(),[extra]:0},owner));}
  for(const bad of [{...batch(),revision:0},{...batch(),ownerId:meeting},{...batch(),historyIssue:"connected"},
    {...batch(),notices:[{...notice(),target:"recorder"}]},{...batch(),notices:[notice(),notice()]},
    {...batch(),notices:[{...notice(),delivery:{...notice().delivery,generation:0}}]},
    {...batch(),notices:[{...notice(),delivery:{...notice().delivery,status:{...notice().delivery!.status,reason:"stream_stopped"}}}]},
    {...batch(),noticeLoss:{microphone:-1,system:0}}])assert.throws(()=>checkedArchivedInputBatch(bad,owner));
});
test("page checks ordered DB ownership, exact cursor and Unicode title bounds",()=>{
  assert.deepEqual(checkedArchivedInputPage(page(),null),page());
  assert.equal(checkedArchivedInputPage(page([{...row(),title:"😀".repeat(120)}]),null).recordings.length,1);
  for(const bad of [page([{...row(),title:"😀".repeat(121)}]),page([row(),row()]),page([row()],owner),page([{...row(),sessionId:meeting}]),
    page([{...row(),issue:"unavailable"}]),{...page(),unknown:true},page(Array.from({length:33},()=>row()))])assert.throws(()=>checkedArchivedInputPage(bad,null));
  assert.throws(()=>checkedArchivedInputPage(page(),owner));
  const rows=Array.from({length:32},(_,i)=>({...row(null),sessionId:id(i+1)}));
  assert.equal(checkedArchivedInputPage(page(rows,id(32)),null).nextCursor,id(32));
  assert.throws(()=>checkedArchivedInputPage(page(rows,id(31)),null));
});
test("independent native revision cannot be compared to live or journal revisions",()=>{
  const s=new ArchivedInputSession(owner);s.accept(batch([notice()],7),0.25);
  assert.equal(s.accept(batch([notice()],6),1),false);
  assert.equal(s.read(1)[0].kind,"interrupted");
  assert.equal(s.accept(batch([notice(2)],8),2),true);assert.equal(s.read(2)[0].kind,"recovered");
});
test("immutable event, claim generation and rendered proof floors survive absence",()=>{
  const s=new ArchivedInputSession(owner);s.accept(batch([notice(1,1,recipient,true)]),0);s.accept(batch([],2),1);
  for(const bad of [notice(1,1,meeting,true),notice(1,2,recipient,false),{...notice(1,1,recipient,true),delivery:{...notice(1,1,recipient,true).delivery!,status:{...notice(1).delivery!.status,sourceEpoch:2}}}])assert.throws(()=>s.accept(batch([bad],3),2));
  assert.equal(s.accept(batch([notice(2)],3),2),true);
});
test("same revision cannot change receipt, diagnostics or loss but can suppress visibility",()=>{
  const s=new ArchivedInputSession(owner);s.accept(batch(),0);s.accept(batch([]),1);s.accept(batch(),2);
  assert.throws(()=>s.accept(batch([notice(1,2)]),3));
  assert.throws(()=>s.accept({...batch(),historyIssue:"missing"},3));
  assert.throws(()=>s.accept({...batch(),noticeLoss:{system:0,microphone:1}},3));
  s.accept({...batch([],2),noticeLoss:{system:0,microphone:1}},4);
  assert.throws(()=>s.accept(batch([],3),5));
  assert.equal(s.accept({...batch([],2),noticeLoss:{microphone:1,system:0}},6),true);
});
test("lease includes request latency; immutable past event has bounded delivery freshness",()=>{
  const s=new ArchivedInputSession(owner);s.accept(batch([notice(1,1,recipient)]),10.5);
  assert.equal(s.read(110.75)[0].delivery!.leaseAgeMs,100.25);
  assert.equal(s.read(1510.5).length,1);assert.deepEqual(s.read(1510.75),[]);
  assert.deepEqual(s.diagnostics(),{loss:{microphone:0,system:0},issue:null});
});
test("reversing or invalid clock permanently disables this archive reader",()=>{
  for(const bad of [9,NaN,Infinity]){const s=new ArchivedInputSession(owner);s.accept(batch(),10);assert.deepEqual(s.read(bad),[]);assert.throws(()=>s.accept(batch([],2),11));}
});
test("controller separates receive, actual rendering and durable close",()=>{
  const c=create();c.acceptPage(page(),null,0);const [receive]=c.paint(0).attempts;
  assert.equal(c.paint(0).views[0].notices.length,0);assert.equal(c.complete(receive,receipt(receive),1),true);
  c.acceptPage(page([row(batch([own(receive)],2))]),null,2);assert.equal(c.paint(2).views[0].notices.length,1);
  c.rendered(owner,receive.attempt.noticeId,1,3);const [render]=c.paint(3).attempts;assert.equal(render.attempt.action,"render");
  c.complete(render,receipt(render),4);c.dismiss(owner,"microphone");const [close]=c.paint(4).attempts;assert.equal(close.attempt.action,"close");
  c.complete(close,receipt(close),5);assert.deepEqual(c.paint(5).views[0].notices,[]);
});
test("failed reads hide presentation, retain pending slots and reject optimistic late display",()=>{
  const c=create();c.acceptPage(page(),null,0);const [receive]=c.paint(0).attempts;c.failedRead();
  c.complete(receive,receipt(receive),1);assert.deepEqual(c.paint(1).views[0].notices,[]);assert.equal(c.paint(1).views[0].unavailable,true);
  c.acceptPage(page([row(batch([own(receive)],2))]),null,2);assert.equal(c.paint(2).views[0].notices.length,1);
  const other=create();other.acceptPage(page(),null,0);other.paint(0);other.failedRead();other.acceptPage(page(),null,1001);
  assert.deepEqual(other.paint(1001).attempts,[]);assert.equal(other.paint(1001).views[0].reception.microphone,"unconfirmed");
});
test("known maintenance pauses claims and requires fresh read after resume",()=>{
  const c=create();c.acceptPage(page(),null,0);const [receive]=c.paint(0).attempts;
  c.pause(true);assert.deepEqual(c.paint(1).attempts,[]);c.complete(receive,receipt(receive),2);
  c.pause(false);assert.deepEqual(c.paint(3).views[0].notices,[]);
  c.acceptPage(page([row(batch([own(receive)],2))]),null,4);assert.equal(c.paint(4).views[0].notices.length,1);
});
test("row failure does not suppress another recording or discard its retained diagnosis",()=>{
  const c=create(),other={...row(),sessionId:id(2),batch:{...batch(),ownerId:id(2),notices:[{...notice(),id:notice().id.replace(owner,id(2))}]}};
  c.acceptPage(page([{...row(),batch:{...batch(),historyIssue:"invalid",noticeLoss:{microphone:4,system:0}}},other]),null,0);
  c.paint(0);c.acceptPage(page([row(null,"unavailable"),other]),null,1);
  const state=c.paint(1);assert.equal(state.views[0].unavailable,true);assert.equal(state.views[0].loss.microphone,4);assert.equal(state.views[0].historyIssue,"invalid");
  assert.equal(state.views[1].unavailable,false);assert.equal(state.views[1].reception.microphone,"pending");
});
test("paged traversal wraps and only a complete successful traversal removes absent rows",()=>{
  const c=create();c.acceptPage(page(),null,0);c.paint(0);
  const legacy=Array.from({length:32},(_,i)=>({...row(null),sessionId:id(i+2)}));
  c.acceptPage(page(legacy,id(33)),null,1);assert.equal(c.after(),id(33));assert.equal(c.paint(1).views.length,1);
  assert.throws(()=>c.acceptPage(page(),null,2));c.failedRead();assert.equal(c.paint(2).views.length,1);
  c.acceptPage(page(legacy,id(33)),null,3);c.acceptPage(page([]),id(33),4);assert.equal(c.after(),null);assert.equal(c.paint(4).views.length,0);
  // A newly closed random UUID behind the old cursor is collected next cycle.
  c.acceptPage(page(),null,5);assert.equal(c.paint(5).views.length,1);
});
test("bounded LRU eviction fences callbacks and late receipts even when owner returns",()=>{
  const c=create();c.acceptPage(page(),null,0);const [old]=c.paint(0).attempts;
  for(let p=0;p<4;p++){
    const rows=Array.from({length:32},(_,i)=>{const sessionId=id(p*32+i+2);return {...row(),sessionId,batch:{...batch(),ownerId:sessionId,notices:[{...notice(),id:notice().id.replace(owner,sessionId)}]}};});
    c.acceptPage(page(rows,rows.at(-1)!.sessionId),c.after(),p+1);
  }
  assert.equal(c.paint(5).views.length,128);assert.equal(c.complete(old,receipt(old),6),false);
  c.acceptPage(page([]),c.after(),7);c.acceptPage(page([row(batch([notice(1,2)],2))]),null,8);
  assert.equal(c.complete(old,receipt(old),9),false);c.rendered(owner,old.attempt.noticeId,1,9);
  const [current]=c.paint(9).attempts;assert.equal(current.attempt.generation,2);assert.equal(current.attempt.action,"receive");
});
test("render callbacks cannot discard a due renewal or mutate a stale generation",()=>{
  const c=create();c.acceptPage(page(),null,0);const [receive]=c.paint(0).attempts;c.complete(receive,receipt(receive),1);
  c.acceptPage(page([row(batch([own(receive)],2))]),null,999);c.paint(999);
  c.rendered(owner,receive.attempt.noticeId,1,1000);const attempts=c.paint(1000).attempts;
  assert.deepEqual(attempts.map(a=>a.attempt.action).sort(),["receive","render"]);
  c.failedRead();c.rendered(owner,receive.attempt.noticeId,1,1001);assert.deepEqual(c.paint(1001).attempts,[]);
});
test("legacy rows do not make a local owner; invalid same-owner rebinding stays unavailable",()=>{
  const c=create();c.acceptPage(page([row(null)]),null,0);assert.equal(c.paint(0).views.length,0);
  c.acceptPage(page(),null,1);c.paint(1);c.acceptPage(page([{...row(),meetingId:id(998)}]),null,2);
  assert.equal(c.paint(2).views[0].unavailable,true);assert.deepEqual(c.paint(2).views[0].notices,[]);
});
test("caller mutation cannot rewrite accepted evidence or retained loss floors",()=>{
  const value={...batch(),noticeLoss:{microphone:2,system:0}},s=new ArchivedInputSession(owner);s.accept(value,0);
  value.notices[0].delivery!.status.sourceEpoch=99;value.noticeLoss.microphone=99;
  assert.equal(s.read(1)[0].delivery!.status.sourceEpoch,1);assert.equal(s.diagnostics()!.loss.microphone,2);
  s.read(2)[0].delivery!.status.sourceEpoch=99;s.diagnostics()!.loss.microphone=99;
  assert.equal(s.read(3)[0].delivery!.status.sourceEpoch,1);assert.equal(s.diagnostics()!.loss.microphone,2);
});
