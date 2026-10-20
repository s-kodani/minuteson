// Synthetic IPC exercises the production progress owner and native event guards.
async page => {
  const check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.getByRole('button',{name:'会議A',exact:true}).waitFor();
  const common=page.getByRole('region',{name:'進行中の処理',exact:true});
  await page.evaluate(async()=>{
    const operationId=crypto.randomUUID();window.monBrowserFixture.operationId=operationId;window.monBrowserFixture.admissionStarted=Date.now();
    window.monBrowserFixture.beginTranscriptionAdmission('meeting-0','会議A',operationId);
  });
  await common.getByText('受付中',{exact:true}).waitFor();
  check(await common.getByRole('progressbar').getAttribute('value')===null,'acceptance must be indeterminate');
  await page.waitForTimeout(2100);
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture;
    f.analysisPreparations=[{meetingId:'meeting-0',meetingTitle:'会議A',operationId:f.operationId,phase:'mix',completed:400,total:1000,startedAtMs:Date.now(),sequence:1}];
    f.endTranscriptionAdmission('meeting-0');
  });
  await common.getByText('会議音声を準備中',{exact:true}).waitFor();
  await common.getByText('40%',{exact:true}).waitFor();
  await page.getByRole('button',{name:'設定',exact:true}).click();
  await common.getByText('40%',{exact:true}).waitFor();
  await page.locator('.sidebar').getByRole('button',{name:'会議一覧',exact:true}).click();
  await page.getByRole('button',{name:'会議B',exact:true}).click();
  await common.getByText('40%',{exact:true}).waitFor();
  await common.getByText('会議A',{exact:true}).waitFor();
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture;
    f.analysisPreparations=[];
    const now=Date.now();
    const job={analysisOperation:{id:f.operationId,startedAtMs:f.admissionStarted},id:crypto.randomUUID(),meetingId:'meeting-0',meetingTitle:'会議A',operationId:f.operationId,kind:'diarization',state:'running',completedUnits:0,totalUnits:1,errorCode:null,outputAnalysisId:null,outputMinutesId:null,pauseReason:null,specHash:'a'.repeat(64),inputHash:'b'.repeat(64),retryOfJobId:null,attempt:1,createdAt:now,updatedAt:now};
    f.jobs=f.notificationJobs=[job];
    f.progressEvent={jobId:job.id,specHash:job.specHash,attempt:1,unitIndex:0,invocationOrder:1,sequence:1,engineBasisPoints:6000,finishing:false};
    await f.emitNotification();await f.emitDiarizationRuntime(f.progressEvent);
  });
  await common.getByText('60%',{exact:true}).waitFor();
  await common.getByText('話者分離 · 特徴抽出中',{exact:true}).waitFor();
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture;
    for(const patch of [{sequence:0,engineBasisPoints:100},{attempt:0,sequence:2},{specHash:'foreign',sequence:3}])await f.emitDiarizationRuntime({...f.progressEvent,...patch});
  });
  await common.getByText('60%',{exact:true}).waitFor();
  await page.evaluate(()=>{const f=window.monBrowserFixture;return f.emitDiarizationRuntime({...f.progressEvent,sequence:4,engineBasisPoints:10000});});
  await common.getByText('話者分離 · 話者をクラスタリング中',{exact:true}).waitFor();
  check(await common.getByRole('progressbar').getAttribute('value')===null,'callback 100% must wait for clustering');
  await page.evaluate(()=>{const f=window.monBrowserFixture;return f.emitDiarizationRuntime({...f.progressEvent,sequence:5,engineBasisPoints:10000,finishing:true});});
  await common.getByText('話者分離 · 結果を保存中',{exact:true}).waitFor();
  const elapsed=await common.getByText(/全体の経過時間/).innerText();
  const time=elapsed.match(/全体の経過時間 (\d+)分(\d+)秒/);
  check(time && Number(time[1])*60+Number(time[2])>=2,'job registration reset the operation elapsed time');
  await page.screenshot({path:'output/playwright/meeting-mix-progress.png'});
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.jobs=f.notificationJobs=[{...f.jobs[0],state:'cancelled',updatedAt:Date.now()}];await f.emitNotification();});
  await common.waitFor({state:'hidden'});
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture;
    const job={...f.jobs[0],id:crypto.randomUUID(),operationId:crypto.randomUUID(),kind:'speaker_completion',state:'running',createdAt:Date.now(),updatedAt:Date.now(),executionProgress:{phase:'references',completed:2,total:10,phaseCompleted:2,phaseTotal:4}};
    f.jobs=f.notificationJobs=[job];await f.emitNotification();
  });
  await common.getByText('50%',{exact:true}).waitFor();
  await common.getByText(/参照音声 2／4区間/).waitFor();
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture;f.jobs=f.notificationJobs=[{...f.jobs[0],updatedAt:Date.now(),executionProgress:{phase:'matching',completed:5,total:10,phaseCompleted:1,phaseTotal:4}}];await f.emitNotification();
  });
  await common.getByText('25%',{exact:true}).waitFor();
  await common.getByText(/発言照合 1／4件/).waitFor();
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture;f.jobs=f.notificationJobs=[{...f.jobs[0],id:crypto.randomUUID(),operationId:crypto.randomUUID(),kind:'voice_matching',updatedAt:Date.now(),executionProgress:{phase:'matching',completed:5,total:8,phaseCompleted:2,phaseTotal:3}}];await f.emitNotification();
  });
  await common.getByText('人物を照合 2／3話者',{exact:true}).waitFor();
  await common.getByText('66%',{exact:true}).waitFor();
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture;f.jobs=f.notificationJobs=[{...f.jobs[0],state:'failed',errorCode:'話者分離に必要なメモリを確保できませんでした',updatedAt:Date.now()}];await f.emitNotification();
  });
  await common.waitFor({state:'hidden'});
  await page.getByText(/必要なメモリを確保できませんでした/).first().waitFor();
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.jobs=f.notificationJobs=[{...f.jobs[0],id:crypto.randomUUID(),operationId:crypto.randomUUID(),analysisOperation:null,state:'running',errorCode:null,createdAt:Date.now(),updatedAt:Date.now(),executionProgress:{phase:'matching',completed:0,total:1,phaseCompleted:0,phaseTotal:1}}];await f.emitNotification();});
  await common.getByText('人物を照合 0／1話者',{exact:true}).waitFor();
  check(await common.getByText(/参照音声|発言照合/).count()===0,'a skipped completion stage appeared');
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.jobs=f.notificationJobs=[{...f.jobs[0],state:'cancelled',updatedAt:Date.now()}];await f.emitNotification();});
  await common.waitFor({state:'hidden'});
  return {passed:13,results:['operation elapsed time survives job registration and phase changes','empty completion stage is omitted before voice matching','immediate admission','frame count progress','settings navigation and meeting switch retain the original operation progress','native feature progress','stale events ignored','callback 100% waits for clustering and saving','cancellation clears progress','completion references use their own denominator','completion matching resets the denominator','registered voice matching counts speakers','failure ends the bar and displays the cause']};
}
