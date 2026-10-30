// Actual UI, synthetic IPC only. No user audio or native model execution.
async page => {
  const check=(v,m)=>{if(!v)throw Error(m);},results=[];
  await page.route('**/index.html',r=>r.fulfill({contentType:'text/html',body:'<div id="root"></div><script type="module" src="/tests/browser-fixture.js"></script>'}));
  const boot=async(initial=false)=>{
    await page.goto('http://127.0.0.1:1420/index.html');await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(initial=>{const f=window.monBrowserFixture;f.seedRegeneration();f.setMeetingSource('meeting-0','audio_import');f.trackPlan={tracks:[{id:'imported-track',kind:'imported',startUs:0,endUs:30000000}]};if(initial){f.currentId=null;f.draft=null;f.candidateVisible=false;f.analysisFlow.stage='configuration';}f.speakerTestPreferences=JSON.stringify([f.preferences,f.diarizationPreferences]);},initial);
    await page.getByRole('button',{name:'会議A',exact:true}).click();
  };
  const dialog=label=>page.getByRole('dialog',{name:label,exact:true});
  const count=()=>page.getByRole('combobox',{name:'会議全体の話者数',exact:true});
  const generate=async(label='文字起こしを再生成')=>{
    await page.getByRole('tabpanel',{name:'文字起こし',exact:true}).getByRole('button',{name:label,exact:true}).click();
    await dialog(label).waitFor();await count().waitFor();
  };
  const accept=async(label='文字起こしを再生成')=>{
    await dialog(label).getByRole('button',{name:label,exact:true}).click();
    await dialog(label).waitFor({state:'hidden'});
    await page.waitForFunction(()=>!!window.monBrowserFixture.regenerationRequest);
  };
  await boot();
  check(await count().count()===0,'speaker selection appears outside its confirmation');
  await generate();check(await count().inputValue()==='','new view was not automatic');
  await count().selectOption('5');await page.getByRole('button',{name:'確認をキャンセル',exact:true}).click();
  check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(c=>c.command==='regenerate_transcription')),'cancel started work');
  await generate();check(await count().inputValue()==='5','cancel discarded the selected count');
  await accept();
  check(await page.evaluate(()=>{const f=window.monBrowserFixture,r=f.regenerationRequest;return r.numSpeakers===5&&!('speakerCounts' in r)&&JSON.stringify([f.preferences,f.diarizationPreferences])===f.speakerTestPreferences;}),'count did not bind to this generation or changed global settings');
  check(await count().count()===0,'processing retained the count selector');
  results.push('confirmation alone owns speaker selection; cancellation preserves five speakers and confirms without changing global settings');

  await boot(true);await generate('文字起こしを生成');await count().selectOption('5');
  await page.evaluate(()=>{window.monBrowserFixture.regenerationLostResponse=true;});
  await accept('文字起こしを生成');
  check(await page.evaluate(()=>{const c=window.monBrowserFixture.calls.filter(c=>c.command==='regenerate_transcription');return c.length===1&&c[0].args.request.expected.target===null&&c[0].args.request.numSpeakers===5;}),'initial generation or lost acknowledgement lost the count or duplicated admission');
  results.push('first generation confirms its selected count and freezes one request even when acknowledgement is lost');

  await boot();await generate();check(await count().inputValue()==='','count leaked to a new meeting view');
  await count().selectOption('4');await accept();
  check(await page.evaluate(()=>window.monBrowserFixture.regenerationRequest.numSpeakers===4),'whole meeting count was not sent');
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.analysisFlow.stage='diarization';f.analysisFlow.diarizationJob={id:'diarizing',kind:'diarization',state:'paused'};await f.emitNotification();});
  await page.locator('.transcript-generation-host button').filter({hasText:'生成を再開'}).waitFor();
  check(await count().count()===0,'paused processing showed settings outside confirmation');
  results.push('new views default to automatic and paused processing retains resume controls without the old tools modal');

  await boot(true);await generate('文字起こしを生成');await accept('文字起こしを生成');
  check(await page.evaluate(()=>window.monBrowserFixture.regenerationRequest.numSpeakers===null),'automatic selection did not send null');
  results.push('automatic first generation explicitly sends numSpeakers null');
  return {passed:results.length,results};
}
