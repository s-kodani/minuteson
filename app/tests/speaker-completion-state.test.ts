import {test} from 'node:test';
import assert from 'node:assert/strict';
import {matchingThreshold} from '../src/speaker-completion-state.ts';
import {verifiedJobOutput,JobNotificationTracker,type NotificationJob} from '../src/job-notification-state.ts';

test('completion settings retain exact increments and reject invalid values',()=>{
  assert.equal(matchingThreshold('0.80'),800);assert.throws(()=>matchingThreshold('0.801'));
});
const job:NotificationJob={id:'j',kind:'speaker_completion',meetingId:'m',meetingTitle:'会議',operationId:'op',inputHash:'a'.repeat(64),specHash:'b'.repeat(64),createdAt:1,updatedAt:2,state:'running',completedUnits:0,totalUnits:1,errorCode:null,outputAnalysisId:null,pauseReason:null,retryOfJobId:null,attempt:1};
test('completion notifications require their exact persisted receipt including zero-change runs',()=>{
  const tracker=new JobNotificationTracker(0);tracker.ingest([job]);
  const done={...job,state:'succeeded',completedUnits:1,updatedAt:3};tracker.ingest([done]);
  const proof={job:done,contentHash:done.specHash,publicationState:'completion',warnings:['0件を補完、1件を見送りました']};
  assert.equal(verifiedJobOutput(done,proof),'completion');
  assert.throws(()=>verifiedJobOutput(done,{...proof,contentHash:'c'.repeat(64)}));
  assert.throws(()=>verifiedJobOutput({...done,kind:'diarization'},{...proof,job:{...done,kind:'diarization'}}));
  assert.ok(tracker.confirmOutput(done,'completion')?.message.includes('話者補完'));
});

test('nearest identification shows low confidence scores and their reason without treating similarity as probability', async()=>{
  const {identificationText}=await import('../src/speaker-completion-state.ts');
  const value={similarityMilli:530,runnerUpMilli:200,needsReview:true,reason:'nearest_below_threshold'};
  const text=identificationText(value)!;
  assert.match(text.label,/要確認/);assert.match(text.label,/0.530/);assert.match(text.detail,/0.330/);assert.match(text.detail,/確率ではありません/);
  assert.doesNotMatch(identificationText({...value,needsReview:false,reason:'applied'})!.label,/要確認/);
  assert.match(identificationText({...value,runnerUpMilli:null})!.detail,/1人/);
  assert.match(identificationText({...value,reason:'nearest_different_winners'})!.detail,/候補が不一致/);
  for(const patch of [{similarityMilli:1001},{similarityMilli:NaN},{runnerUpMilli:600},{runnerUpMilli:-1001}])assert.equal(identificationText({...value,...patch}),null);
});
