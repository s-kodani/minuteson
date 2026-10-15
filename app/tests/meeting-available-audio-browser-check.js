// Real UI, synthetic metadata and PCM only. No user audio or recording.
async page => {
  const check=(v,m)=>{if(!v)throw Error(m);};
  await page.route('**/index.html',r=>r.fulfill({contentType:'text/html',body:'<div id="root"></div><script type="module" src="/tests/browser-fixture.js"></script>'}));
  const results=[];
  for(const modes of [["mic"],["system"],["mic","system","both"],["imported"],[]]){
    await page.goto('http://localhost:1420/index.html');
    await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(m=>{const f=window.monBrowserFixture;f.meetingPlaybackModes=m;if(m.includes('imported'))f.setMeetingSource('meeting-0','audio_import');},modes);
    await page.getByRole('button',{name:'会議A',exact:true}).click();
    const play=page.getByRole('group',{name:'会議全体の音声再生',exact:true}).getByRole('button',{name:'再生',exact:true});
    if(!modes.length){
      await page.getByRole('alert').filter({hasText:'再生できる確定済み音声がありません'}).waitFor();
      check(await play.isDisabled(),'missing sources remained playable');
      check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(c=>c.command==='prepare_meeting_playback')),'missing sources prepared playback');
      results.push('no saved tracks stay disabled without fallback');continue;
    }
    await play.waitFor();
    await page.waitForFunction(()=>!document.querySelector('.audio-controls button')?.disabled);
    await play.click();
    await page.getByRole('button',{name:'一時停止',exact:true}).waitFor();
    const expected=modes.includes('both')?'both':modes[0];
    check(await page.evaluate(mode=>window.monBrowserFixture.calls.filter(c=>c.command==='prepare_meeting_playback').at(-1).args.request.mode===mode,expected),'wrong initial track requested');
    await page.getByRole('button',{name:'一時停止',exact:true}).click();
    await page.getByRole('button',{name:'再生設定',exact:true}).click();
    const select=page.getByRole('combobox',{name:'会議全体の再生トラック',exact:true});
    await select.waitFor();
    const options=await select.locator('option').evaluateAll(items=>items.map(e=>e.value));
    check(JSON.stringify(options)===JSON.stringify(modes),'unavailable track offered');
    if(expected==='both'){
      await select.selectOption('system');
      await page.waitForFunction(()=>window.monBrowserFixture.calls.filter(c=>c.command==='prepare_meeting_playback').at(-1).args.request.mode==='system');
      results.push('explicit switch still requests the selected saved track');
    }
    results.push(`${expected} default and available options`);
  }
  await page.goto('http://localhost:1420/index.html');await page.waitForFunction(()=>!!window.monBrowserFixture);
  await page.evaluate(()=>{window.monBrowserFixture.meetingSummaryFailure=true;});
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'模擬の保存音声確認失敗'}).waitFor();
  check(await page.getByRole('group',{name:'会議全体の音声再生',exact:true}).getByRole('button',{name:'再生',exact:true}).isDisabled(),'metadata failure permitted playback');
  results.push('metadata failure cannot prepare an assumed track');
  await page.goto('http://localhost:1420/index.html');await page.waitForFunction(()=>!!window.monBrowserFixture);
  await page.evaluate(()=>{window.monBrowserFixture.meetingAudio.failModes=['both'];});
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  await page.getByRole('group',{name:'会議全体の音声再生',exact:true}).getByRole('button',{name:'再生',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'指定した会議音声に欠損があります'}).waitFor();
  check(await page.evaluate(()=>{const c=window.monBrowserFixture.calls;return c.filter(c=>c.command==='prepare_meeting_playback').length===1&&!c.some(c=>c.command==='read_playback_chunk');}),'native validation failure fell back or read PCM');
  results.push('native corruption rejection remains an error without fallback');
  return {passed:results.length,results};
}
