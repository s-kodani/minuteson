// Production UI with synthetic ownership/PCM. No microphones or user files.
async (page,capturePath)=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();
    const body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const panel=()=>page.getByRole('dialog',{name:'録音前の入力確認',exact:true});
  const close=()=>panel().getByRole('button',{name:'確認を閉じる',exact:true});
  const count=command=>page.evaluate(command=>window.closeFixture.calls.filter(c=>c.command===command).length,command);
  const boot=async(query='')=>{
    await page.goto('http://127.0.0.1:1420/'+query);await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole('button',{name:'新しい録音',exact:true}).click();
    await page.getByRole('region',{name:'録音前の5秒テスト',exact:true}).waitFor();
  };
  const begin=async()=>{
    await page.locator('.capture-test-panel > button').click();
    const consent=page.getByRole('dialog',{name:'5秒の録音前テストを開始しますか？',exact:true});
    await consent.getByRole('button',{name:'5秒テストを開始する',exact:true}).click();
    await consent.waitFor({state:'hidden'});await panel().waitFor();
  };
  const ready=async()=>{await page.evaluate(()=>window.closeFixture.testReady());await panel().getByText('停止と音声の検証が完了しました',{exact:true}).waitFor();};
  const released=()=>page.waitForFunction(()=>!window.closeFixture.captureTest&&Object.keys(window.closeFixture.testPlayback.tokens).length===0&&Object.keys(window.closeFixture.testPlayback.reservations).length===0);
  try{
    for(const mode of ['button','escape','outside']){
      await boot();await begin();await ready();
      check(await panel().getByRole('button',{name:'テスト音声を破棄',exact:true}).count()===0,'manual discard remains');
      await panel().getByText('この画面を閉じると、テスト音声は自動で削除されます。',{exact:true}).waitFor();
      if(mode==='button')await close().click();
      else if(mode==='escape')await page.keyboard.press('Escape');
      else await page.mouse.click(2,2);
      await panel().waitFor({state:'hidden'});await released();
      check(await count('discard_capture_test')===1&&await count('cancel_capture_test')===0,'terminal close did not release exactly once');
      await page.getByRole('button',{name:'会議一覧',exact:true}).first().click();await page.getByRole('heading',{name:'会議一覧',exact:true}).waitFor();
      check(await count('start_recording')===0,'test created a meeting recording');
      results.push(`${mode}: closing deletes test audio and unlocks navigation without manual discard`);
    }
    for(const phase of ['preparing','recording','checking']){
      await boot();await begin();
      if(phase!=='preparing')await page.evaluate(phase=>window.closeFixture.testChanged({phase}),phase);
      await close().click();await panel().getByText('5秒録音テストを終了しています。完了するまでお待ちください。',{exact:true}).waitFor();
      check(await count('cancel_capture_test')===1&&await count('discard_capture_test')===0&&await close().isDisabled(),'cleanup did not wait for capture exit');
      await page.evaluate(()=>window.closeFixture.testChanged({phase:'cancelled',busy:false}));
      await panel().waitFor({state:'hidden'});await released();
      results.push(`${phase}: close waits for the same capture process before deleting`);
    }
    await boot();await begin();await ready();
    await panel().getByRole('button',{name:'マイクのテスト音声を再生',exact:true}).click();
    await panel().getByRole('button',{name:'一時停止',exact:true}).waitFor();
    await close().click();await panel().waitFor({state:'hidden'});await released();
    check(await count('close_playback')>=1&&await count('discard_capture_test')===1,'playback release did not precede deletion');
    results.push('playing test audio is stopped and its token released before deletion');

    await boot();await begin();await ready();await page.evaluate(()=>window.closeFixture.testPlayback.hold=true);
    await panel().getByRole('button',{name:'マイクのテスト音声を再生',exact:true}).click();
    await page.waitForFunction(()=>window.closeFixture.testPlayback.pending.length===1);
    await close().click();await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==='cancel_playback_preparation'));
    check(await count('discard_capture_test')===0,'deleted audio before pending preparation completed');
    await page.evaluate(()=>{for(const resolve of window.closeFixture.testPlayback.pending.splice(0))resolve();});
    await panel().waitFor({state:'hidden'});await released();
    check(await count('cancel_playback_preparation')>=1&&await count('read_playback_chunk')===0,'pending playback prepared after close');
    results.push('close cancels pending playback preparation and releases reservations');

    await boot();await begin();await ready();await page.evaluate(()=>window.closeFixture.testDiscardFailure=true);
    await close().click();await panel().getByRole('alert').filter({hasText:'「閉じる」'}).first().waitFor();
    check(await panel().isVisible()&&await close().isEnabled()&&await page.evaluate(()=>!!window.closeFixture.captureTest),'failed cleanup hid the owner');
    await page.evaluate(()=>window.closeFixture.testDiscardFailure=false);await close().click();await panel().waitFor({state:'hidden'});await released();
    results.push('cleanup failures remain inline and closing retries the same test');

    await boot();await begin();await ready();await page.evaluate(()=>window.closeFixture.testDiscardLost=true);
    await close().click();await panel().waitFor({state:'hidden'});await released();
    check(await count('discard_capture_test')===1,'lost deletion reply replayed disposal');
    results.push('lost deletion receipt is reconciled against empty ownership');

    await boot();await begin();await ready();await panel().getByRole('button',{name:'もう一度5秒録音して確認',exact:true}).click();
    await page.getByRole('dialog',{name:'5秒の録音前テストを開始しますか？',exact:true}).getByRole('button',{name:'5秒テストを開始する',exact:true}).click();
    await page.waitForFunction(()=>window.closeFixture.calls.filter(c=>c.command==='start_capture_test').length===2);
    const ids=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==='start_capture_test').map(c=>c.args.request.runId));
    check(ids[0]!==ids[1]&&await count('discard_capture_test')===1,'retest reused the capture identity');
    await page.evaluate(()=>window.closeFixture.testReady());await ready();
    for(const width of [1200,390]){
      await page.setViewportSize({width,height:800});
      check(await panel().evaluate(e=>e.scrollWidth<=e.clientWidth+1),'cleanup dialog overflows');
      await panel().screenshot({path:capturePath(`test-auto-cleanup-${width}.png`)});
    }
    await close().click();await panel().waitFor({state:'hidden'});await released();results.push('retest releases its predecessor and both dialog sizes fit');
    await page.setViewportSize({width:1200,height:900});
    await page.evaluate(()=>{
      const meeting=window.dataFixture.meetings[0];meeting.lifecycle='recorded';meeting.sourceType='recording';
      window.closeFixture.regenerationFlow={id:'previous-flow',meetingId:meeting.id,stage:'review',lockVersion:1,transcriptionJob:null,diarizationJob:null,errorMessage:null};
    });
    await page.getByRole('button',{name:'会議一覧',exact:true}).first().click();
    await page.getByRole('button',{name:'既存会議',exact:true}).click();
    const regenerate=page.locator('.transcript-generation-host button').filter({hasText:'文字起こしを再生成'});
    await regenerate.click();
    await page.getByRole('dialog',{name:'文字起こしを再生成',exact:true}).getByRole('button',{name:'文字起こしを再生成',exact:true}).click();
    await page.waitForFunction(()=>!!window.closeFixture.regenerationRequest);
    check(await page.evaluate(()=>!window.closeFixture.captureTest),'regeneration retained test audio');
    results.push('after closing the five-second test, transcript regeneration is admitted without a discard operation');
    for(const platform of ['windows','macos']){
      await boot(`?resident=${platform}`);await begin();await ready();
      await page.evaluate(()=>window.closeFixture.request(false));
      await page.waitForFunction(()=>window.closeFixture.hidden);
      await released();
      check(await count('discard_capture_test')===1,'backgrounding retained test audio');
      check(await page.getByRole('dialog',{name:'録音前テストを閉じて終了しますか？',exact:true}).count()===0,'extra test deletion confirmation remains');
      results.push(`${platform}: backgrounding automatically releases a ready test without a deletion confirmation`);
      await boot(`?resident=${platform}`);await begin();
      await page.evaluate(()=>window.closeFixture.request(true));
      await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==='cancel_capture_test'));
      check(await count('finish_main_close')===0,'application closed before test capture exited');
      await page.evaluate(()=>window.closeFixture.testChanged({phase:'cancelled',busy:false}));
      await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==='finish_main_close'));
      await released();results.push(`${platform}: quitting stops an active test and deletes its audio before application close`);
    }
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator('body').innerText()}`);}
}
