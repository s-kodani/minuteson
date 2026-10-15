// Actual UI, synthetic IPC only. No user audio or native model execution.
async page => {
  const check=(v,m)=>{if(!v)throw Error(m);},results=[];
  await page.route('**/index.html',r=>r.fulfill({contentType:'text/html',body:'<div id="root"></div><script type="module" src="/tests/browser-fixture.js"></script>'}));
  const boot=async(initial=false)=>{
    await page.goto('http://localhost:1420/index.html');await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(initial=>{const f=window.monBrowserFixture;f.seedRegeneration();f.setMeetingSource('meeting-0','audio_import');f.trackPlan={tracks:[{id:'imported-track',kind:'imported',startUs:0,endUs:30000000}]};if(initial){f.currentId=null;f.draft=null;f.candidateVisible=false;f.analysisFlow.stage='configuration';}f.speakerTestPreferences=JSON.stringify([f.preferences,f.diarizationPreferences]);},initial);
    await page.getByRole('button',{name:'会議A',exact:true}).click();
    await page.getByRole('button',{name:'会議管理・録音・解析操作',exact:true}).click();
    await page.getByRole('combobox',{name:'話者数',exact:true}).waitFor();
  };
  const dialog=()=>page.getByRole('dialog',{name:'会議管理・録音・解析操作',exact:true});
  await boot();
  const select=page.getByRole('combobox',{name:'話者数',exact:true});
  check(await select.inputValue()==='','new view was not automatic');
  await select.selectOption('5');await dialog().getByRole('button',{name:'文字起こしを再生成',exact:true}).click();
  await page.getByRole('button',{name:'確認をキャンセル',exact:true}).click();
  check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(c=>c.command==='regenerate_transcription')),'cancel started work');results.push('optional count defaults to auto and cancel performs no generation');
  await dialog().getByRole('button',{name:'文字起こしを再生成',exact:true}).click();
  await page.getByRole('button',{name:'文字起こしを再生成',exact:true}).click();
  await page.waitForFunction(()=>!!window.monBrowserFixture.regenerationRequest);
  check(await page.evaluate(()=>{const f=window.monBrowserFixture,r=f.regenerationRequest;return r.speakerCounts.length===1&&r.speakerCounts[0].trackId==='imported-track'&&r.speakerCounts[0].numSpeakers===5&&JSON.stringify([f.preferences,f.diarizationPreferences])===f.speakerTestPreferences;}),'count did not bind to the requested generation or changed global settings');
  check(await select.isDisabled(),'running count remained editable');results.push('five speakers belong to this generation only and global model settings remain unchanged');
  await boot(true);await select.selectOption('5');
  await page.evaluate(()=>{window.monBrowserFixture.regenerationLostResponse=true;});
  await dialog().getByRole('button',{name:'文字起こしを生成',exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==='transcription');
  check(await page.evaluate(()=>{const c=window.monBrowserFixture.calls.filter(c=>c.command==='regenerate_transcription');return c.length===1&&c[0].args.request.expected.target===null&&c[0].args.request.speakerCounts[0].numSpeakers===5;}),'initial generation or lost acknowledgement lost the count or duplicated admission');
  results.push('first generation freezes the count once even when its acknowledgement is lost');
  await page.goto('http://localhost:1420/index.html');await page.waitForFunction(()=>!!window.monBrowserFixture);
  await page.getByRole('button',{name:'会議A',exact:true}).click();await page.getByRole('button',{name:'会議管理・録音・解析操作',exact:true}).click();
  const mic=page.getByRole('combobox',{name:'マイクの話者数',exact:true}),system=page.getByRole('combobox',{name:'PC音声の話者数',exact:true});await mic.waitFor();
  check(await mic.inputValue()===''&&await system.inputValue()==='','count leaked to another operation or recording track');
  results.push('recording lanes have independent automatic defaults');
  await mic.selectOption('1');await system.selectOption('4');
  await dialog().getByRole('button',{name:'文字起こしを再生成',exact:true}).click();
  await page.getByRole('button',{name:'文字起こしを再生成',exact:true}).click();
  await page.waitForFunction(()=>!!window.monBrowserFixture.regenerationRequest);
  check(await page.evaluate(()=>{const v=window.monBrowserFixture.regenerationRequest.speakerCounts;return v.length===2&&v.some(c=>c.trackId==='microphone'&&c.numSpeakers===1)&&v.some(c=>c.trackId==='system'&&c.numSpeakers===4);}), 'recording counts were applied to the wrong lane');
  results.push('microphone and PC counts are sent to their own saved tracks');
  return {passed:results.length,results};
}
