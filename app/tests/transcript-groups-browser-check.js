// Production UI with synthetic persisted revisions; native SQLite has separate tests.
async (page,capturePath) => {
  const results=[],check=(value,message)=>{if(!value)throw Error(message);};
  const nav=page.getByRole('navigation',{name:'メインナビゲーション',exact:true});
  const editor=page.getByRole('region',{name:'文字起こし編集',exact:true});
  const body=editor.getByRole('region',{name:'発言本文',exact:true});
  const waitUntilEditable=()=>page.waitForFunction(()=>{
    const body=document.querySelector('.transcript-body-edit');
    return body && !body.closest('[inert]');
  });
  await page.evaluate(()=>{
    const row=(id,startUs,text,speakerIds=['speaker-a'])=>({id,trackId:'microphone',startUs,endUs:startUs+1_000_000,recognizedText:text,effectiveText:text,speakerIds,attributionState:'assigned'});
    const fixture=window.monBrowserFixture;
    fixture.setUtterances([
      row('speech-a',0,'本日は'),row('speech-c',1_000_000,'点検日です。'),row('speech-d',3_000_000,'よろしくお願いします。'),
      row('speech-b',4_000_000,'はい。',['speaker-b']),row('speech-e',5_000_000,'続いて確認します。')
    ]);
    fixture.seedRegeneration();
    fixture.analysisFlow.diarizationJob={state:"succeeded",outputAnalysisId:fixture.currentId};
    let beginConsolidation;
    const consolidationStarted=new Promise(resolve=>{beginConsolidation=resolve;});
    const invoke=window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke=async (command,args)=>{
      if(command==="consolidate_transcript"&&!fixture.releaseOpenConsolidation){
        await new Promise(resolve=>{fixture.releaseOpenConsolidation=resolve;beginConsolidation();});
      }
      if(command==="get_analysis_flow"){
        await consolidationStarted;
        fixture.openFlowReads=(fixture.openFlowReads??0)+1;
      }
      return invoke(command,args);
    };
  });
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.openFlowReads>=2);
  const verifying=page.getByRole('status').filter({hasText:'文字起こしの状態を確認中です。'});
  check(await verifying.count()===0,'automatic consolidation was shown as a read failure');
  check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(({command,args})=>command==='record_meeting_open_failure'&&args.step==='flush_pending_transcript_edits')),'expected busy state was logged as a failure');
  await page.evaluate(()=>window.monBrowserFixture.releaseOpenConsolidation());
  await page.waitForFunction(()=>window.monBrowserFixture.consolidationHistory?.length===1&&document.querySelectorAll('[data-utterance-id]').length===3);
  await waitUntilEditable();
  await page.waitForFunction(()=>window.monBrowserFixture.openFlowReads>=3);
  check(await verifying.count()===0,'analysis refresh did not recover after consolidation');
  check(await editor.getByRole('button',{name:/区間を表示|区間をまとめる/}).count()===0,'retired disclosure remains');
  check(await body.locator('.utterance-group-heading,.utterance-group-fragment').count()===0,'display still retains fragment rows');
  check((await body.innerText()).includes('本日は点検日です。よろしくお願いします。'),'consolidation lost source text');
  const firstId=await body.locator('article').first().getAttribute('data-utterance-id');
  const row=()=>body.locator(`[data-utterance-id="${firstId}"]`);
  check(await page.evaluate(()=>window.monBrowserFixture.consolidationHistory[0].utterances.length===5),'previous revision did not retain its source records');
  results.push('continuous source records persist as one ordinary utterance and all disclosure controls disappear');

  await row().getByRole('button',{name:'00:00:00の発言を編集',exact:true}).click();
  const input=row().getByRole('textbox',{name:'00:00:00の発言',exact:true});
  check(await input.inputValue()==='本日は点検日です。よろしくお願いします。','editing did not open the entire merged utterance');
  await input.fill('本日は修正した点検日です。よろしくお願いします。');
  await page.evaluate(()=>{window.monBrowserFixture.failSave=true;});
  await row().getByRole('button',{name:'保存',exact:true}).click();
  await editor.getByRole('button',{name:'保存を再試行',exact:true}).waitFor();
  check(await input.inputValue()==='本日は修正した点検日です。よろしくお願いします。','failed save lost merged text');
  await page.evaluate(()=>{window.monBrowserFixture.failSave=false;});
  await editor.getByRole('button',{name:'保存を再試行',exact:true}).click();await input.waitFor({state:'hidden'});
  check(await page.evaluate(id=>{const edits=window.monBrowserFixture.draft.payload.utterances;return edits.length===1&&edits[0].utterance_id===id;},firstId),'whole-text edit wrote separate source fragments');
  results.push('the combined body edits and saves as one record and survives failed saving');

  await row().getByRole('button',{name:'音声を再生',exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==='prepare_meeting_playback'));
  check(await page.evaluate(id=>{const r=window.monBrowserFixture.calls.filter(c=>c.command==='prepare_meeting_playback').at(-1).args.request.startReference;return r.kind==='utterance'&&r.utteranceId===id&&r.startUs===0&&r.endUs===4_000_000;},firstId),'playback still references only the first former fragment');
  results.push('the ordinary playback action uses the saved merged utterance and its complete time range');

  await row().getByRole('button',{name:'00:00:00の話者を変更',exact:true}).click();
  const dialog=page.getByRole('dialog',{name:'00:00:00の話者を変更',exact:true});
  await dialog.getByRole('checkbox',{name:'佐藤',exact:true}).check();
  check(await dialog.isVisible(),'speaker checkbox closed the dialog');
  await dialog.getByRole('button',{name:'保存',exact:true}).click();await dialog.waitFor({state:'hidden'});
  check(await page.evaluate(id=>window.monBrowserFixture.draft.payload.utterances.find(u=>u.utterance_id===id).speaker_ids.length===2,firstId),'assignment did not apply to the combined utterance');
  results.push('speaker assignment applies to the entire merged utterance only on Save');

  const search=editor.getByRole('searchbox',{name:'発言を検索',exact:true});
  await search.fill('点検日です。よろしく');
  check(await body.getByRole('article').count()===1,'search cannot match across former source boundaries');
  await search.fill('はい。');
  await page.evaluate(async id=>{(await import('/src/playback-follow.ts')).playbackTimeline.publish({owner:'consolidation-check',session:'check',meetingId:'meeting-0',analysisId:window.monBrowserFixture.currentId,utteranceId:id,scope:'utterance',timeUs:3_500_000,playing:true,trackKind:'mic'});},firstId);
  await page.locator('.transcript-primary-heading').getByRole('button',{name:'再生位置へ移動',exact:true}).click();
  await row().waitFor();check(await search.inputValue()===''&&await body.getByRole('article').count()===3,'jump did not reveal the merged playback row');
  await page.evaluate(async()=>{(await import('/src/playback-follow.ts')).playbackTimeline.release('consolidation-check');});
  for(const width of [1280,390]){
    await page.setViewportSize({width,height:900});
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'consolidated rows overflow');
    await page.screenshot({path:capturePath(`consolidated-${width}.png`)});
  }
  await page.setViewportSize({width:1280,height:900});
  await nav.getByRole('button',{name:'会議一覧',exact:true}).click();
  await page.getByRole('button',{name:'会議A',exact:true}).click();await row().waitFor();
  check(await body.getByRole('article').count()===3,'reopening restored source fragments');
  results.push('search spans former boundaries; playback jump, reopening and narrow layouts use ordinary rows');

  await editor.getByRole('button',{name:'話者を統合',exact:true}).click();
  const merge=page.getByRole('dialog',{name:'話者の統合',exact:true});
  await merge.getByRole('combobox',{name:'統合元',exact:true}).selectOption({label:'田中'});
  await merge.getByRole('combobox',{name:'統合先',exact:true}).selectOption({label:'佐藤'});
  await merge.getByRole('button',{name:'統合して保存',exact:true}).click();
  await merge.waitFor({state:'hidden'});
  await page.waitForFunction(()=>window.monBrowserFixture.consolidationHistory.length===2&&document.querySelectorAll('[data-utterance-id]').length===1);
  await waitUntilEditable();
  check(await editor.getByRole('button',{name:'田中の名前を変更',exact:true}).count()===0&&await editor.getByRole('button',{name:'佐藤の名前を変更',exact:true}).count()===1,'persisting turns resurrected the merged source speaker');
  results.push('a saved speaker merge can combine adjacent turns without resurrecting the source speaker');

  await body.getByRole('button',{name:'00:00:00の話者を変更',exact:true}).click();
  await dialog.getByRole('button',{name:'発言を手動分割',exact:true}).click();
  const split=page.getByRole('dialog',{name:'発言の手動分割',exact:true});
  await split.getByRole('button',{name:'分割して保存',exact:true}).click();await split.waitFor({state:'hidden'});await dialog.waitFor({state:'hidden'});
  await page.waitForFunction(()=>document.querySelectorAll('[data-utterance-id^="split:"]').length===2);
  await waitUntilEditable();
  check(await body.getByRole('article').count()===2&&await page.evaluate(()=>window.monBrowserFixture.consolidationHistory.length)===2,'manual parts with the same speaker were automatically remerged');
  results.push('manual splitting remains available on a consolidated utterance and its two saved parts stay separate');

  await nav.getByRole('button',{name:'会議一覧',exact:true}).click();
  const before=await page.evaluate(async()=>{
    const f=window.monBrowserFixture,current=await window.__TAURI_INTERNALS__.invoke('get_current_analysis',{meetingId:'meeting-0'}),speaker=current.speakers[0].id;
    f.draft=null;f.failConsolidation=true;
    f.setUtterances([0,1].map(i=>({id:`new-${i}`,trackId:'microphone',startUs:i*1_000_000,endUs:(i+1)*1_000_000,recognizedText:`再確認${i}`,effectiveText:`再確認${i}`,speakerIds:[speaker],attributionState:'assigned'})));
    return f.consolidationHistory.length;
  });
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  const retry=editor.getByRole('button',{name:'発言統合を再試行',exact:true});await retry.waitFor();
  check(await body.getByRole('article').count()===2,'rejected consolidation hid the original records');
  await page.evaluate(()=>{window.monBrowserFixture.failConsolidation=false;window.monBrowserFixture.loseConsolidationReply=true;});
  await retry.click();
  await page.waitForFunction(expected=>window.monBrowserFixture.consolidationHistory.length===expected,before+1);
  await retry.waitFor();await retry.click();
  await page.waitForFunction(()=>document.querySelectorAll('[data-utterance-id]').length===1);
  await waitUntilEditable();
  check(await page.evaluate(expected=>window.monBrowserFixture.consolidationHistory.length===expected,before+1),'lost receipt created duplicate stored revisions');
  results.push('failed consolidation keeps the originals; a lost receipt retries the same operation without duplicating revisions');
  return {passed:results.length,results};
}
