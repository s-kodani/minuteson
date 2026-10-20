import test from 'node:test';
import assert from 'node:assert/strict';
import { Admissions } from '../src/transcription-admission.ts';

test('admission is immediate and keeps elapsed time through job registration',()=>{
  const a=new Admissions();const op=a.begin('meeting','会議','operation');
  const start=a.getSnapshot()[0].startedAtMs;
  assert.equal(a.getSnapshot()[0].phase,'accepting');
  a.end('meeting',op);assert.equal(a.getSnapshot()[0].phase,'waiting');
  a.observe([{meetingId:'meeting',kind:'transcription',createdAt:start-2000}]);
  assert.equal(a.getSnapshot().length,1);
  a.observe([{meetingId:'meeting',kind:'transcription',createdAt:start,analysisOperation:{id:'operation'}}]);
  assert.equal(a.getSnapshot().length,0);assert.equal(a.starts.get('meeting'),start);
});

test('late acknowledgement from a different operation cannot end or clear a new admission',()=>{
  const a=new Admissions();a.begin('meeting','会議','first');a.begin('meeting','会議','second');
  a.end('meeting','first');a.clear('meeting','first');
  assert.equal(a.getSnapshot()[0].operationId,'second');assert.equal(a.getSnapshot()[0].phase,'accepting');
});

test('a read started before acceptance cannot clear a preparation; a settled empty read can',()=>{
  const a=new Admissions();a.begin('meeting','会議','operation');a.end('meeting','operation');
  const p=a.getSnapshot()[0];
  a.observePreparations([],p.acceptedAtMs!-1);assert.equal(a.getSnapshot().length,1);
  a.observePreparations([{...p,phase:'verifying',sequence:1}],p.acceptedAtMs!+1);assert.equal(a.getSnapshot().length,1);
  a.observePreparations([],p.acceptedAtMs!+1);assert.equal(a.getSnapshot().length,0);
});

test('a newer unrelated job cannot claim preparation ownership',()=>{
 const a=new Admissions();a.begin('meeting','会議','pending');
 a.observe([{meetingId:'meeting',kind:'transcription',createdAt:Date.now()+10000,analysisOperation:{id:'foreign'}}]);
 assert.equal(a.getSnapshot().length,1);
 assert.equal(a.bind('meeting','foreign','new-flow'),'foreign');
 assert.equal(a.bind('meeting','pending','actual-flow'),'actual-flow');
 a.observe([{meetingId:'meeting',kind:'transcription',operationId:'flow:actual-flow:transcription'}]);
 assert.equal(a.getSnapshot().length,0);
});
