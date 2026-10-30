// Production UI with synthetic IPC. Never launches a native recorder.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();
    await route.fulfill({response,body:(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>')});
  });
  const workspace=()=>page.getByRole('region',{name:'録音状態',exact:true});
  const count=command=>page.evaluate(command=>window.closeFixture.calls.filter(c=>c.command===command).length,command);
  const testDialog=()=>page.getByRole('dialog',{name:'録音前の入力確認',exact:true});
  const testButton=()=>workspace().locator('.capture-test-panel>button');
  const closeTest=()=>testDialog().getByRole('button',{name:'確認を閉じる',exact:true}).click();
  const scope=()=>workspace().getByRole('combobox',{name:'PC音声の取得範囲',exact:true});
  const capture=async name=>page.screenshot({path:`output/ui-compact-proposal/actual-${name}.png`});
  const fits=async(desktop=true)=>{
    const bounds=await workspace().evaluate(e=>{
      const rect=e.getBoundingClientRect(),port=e.closest('.content');
      const parent=port.getBoundingClientRect(),css=getComputedStyle(port);
      return {width:rect.width,left:rect.left,right:rect.right,contentLeft:parent.left+parseFloat(css.paddingLeft),contentRight:parent.left+port.clientWidth-parseFloat(css.paddingRight),bottom:rect.bottom,viewport:innerHeight,overflow:e.scrollWidth>e.clientWidth+1,pageOverflow:document.documentElement.scrollWidth>innerWidth+1,scrolls:port.scrollHeight>port.clientHeight+1};
    });
    check(Math.abs(bounds.left-bounds.contentLeft)<1&&Math.abs(bounds.right-bounds.contentRight)<1,`recording card does not fill shared content width ${JSON.stringify(bounds)}`);
    check(!bounds.overflow&&!bounds.pageOverflow,`horizontal overflow ${JSON.stringify(bounds)}`);
    if(desktop)check(!bounds.scrolls&&bounds.bottom<=bounds.viewport,`desktop scroll ${JSON.stringify(bounds)}`);
    const selects=await workspace().getByRole('combobox').evaluateAll(fields=>fields.map(field=>{
      const css=getComputedStyle(field);return {arrow:css.backgroundImage,padding:parseFloat(css.paddingRight),cursor:css.cursor,disabled:field.disabled};
    }));
    check(selects.every(field=>field.arrow!=='none'&&field.padding>=40&&field.cursor===(field.disabled?'not-allowed':'pointer')),'recording select lost its visible arrow or dropdown affordance');
  };
  const boot=async()=>{
    await page.goto('http://127.0.0.1:1420/');await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole('button',{name:'新しい録音',exact:true}).click();
    await workspace().getByRole('button',{name:'録音を開始',exact:true}).waitFor();
    await page.waitForFunction(()=>!document.querySelector('#recording-title')?.disabled&&!document.querySelector('[aria-label="録音するマイク"]')?.disabled);
  };
  await boot();
  check(await count('start_capture_test')===0&&await count('start_recording')===0,'preparation started capture');
  check(await workspace().getByRole('button',{name:'録音の詳細設定',exact:true}).count()===0,'duplicate settings trigger');
  for(const [width,height] of [[1280,720],[1366,768]])for(const theme of ['light','dark']){
    await page.setViewportSize({width,height});await page.evaluate(t=>document.documentElement.dataset.theme=t,theme);
    await scope().selectOption('system');await fits();await capture(`new-${width}-${theme}`);
    await scope().selectOption('application');
    await page.waitForFunction(()=>{const e=document.querySelector('[aria-label="録音するアプリ"]');return e&&!e.disabled;});
    if(await page.getByRole('combobox',{name:'録音するアプリ',exact:true}).inputValue()==='')check(await workspace().getByRole('button',{name:'録音を開始',exact:true}).isDisabled(),'unselected app allowed start');
    const key=await page.evaluate(()=>{const a=window.closeFixture.captureApplications.applications[0];return JSON.stringify([a.pid,a.bundleIdentifier,a.launchTimeUs]);});
    await page.getByRole('combobox',{name:'録音するアプリ',exact:true}).selectOption(key);
    check(await workspace().getByRole('button',{name:'録音を開始',exact:true}).isEnabled(),'selected app cannot start');
    await fits();await capture(`application-${width}-${theme}`);
  }
  results.push('whole-PC and selected-application preparation fit 1280×720 / 1366×768 in both themes without scrolling; explicit selection gates start');
  await page.setViewportSize({width:1280,height:720});await scope().selectOption('system');
  await testButton().click();
  const consent=page.getByRole('dialog',{name:'5秒の録音前テストを開始しますか？',exact:true});
  await consent.waitFor();check(await count('start_capture_test')===0,'test started before consent');
  check(await page.locator('dialog[open]').count()===1&&await page.locator('.capture-test-dialog[open]').count()===0,'initial consent opened an empty test dialog underneath');
  for(const dismiss of ['escape','close','backdrop']){
    if(dismiss==='escape')await page.keyboard.press('Escape');
    else if(dismiss==='close')await consent.getByRole('button',{name:'確認をキャンセル',exact:true}).click();
    else await page.mouse.click(1,1);
    await consent.waitFor({state:'hidden'});
    check(await page.locator('dialog[open]').count()===0&&await count('start_capture_test')===0,'declining initial consent left another test dialog open or started capture');
    await page.waitForFunction(()=>document.activeElement===document.querySelector('.capture-test-panel>button'));
    await testButton().click();await consent.waitFor();
  }
  results.push('initial test opens only consent; Escape, close and backdrop return directly to its trigger without another modal or capture');
  await consent.getByRole('button',{name:'5秒テストを開始する',exact:true}).dblclick();
  await testDialog().waitFor();await testDialog().getByRole('button',{name:'テストを取り消す',exact:true}).waitFor();
  check(await count('start_capture_test')===1&&await page.locator('dialog[open]').count()===1,'test sent more than once or opened duplicate progress dialogs');
  check(await page.getByRole('alert').filter({hasText:'開始要求の受付結果が未確認'}).count()===0,'normal start published an uncertain-result error that obscures the dialog');
  await fits();
  await page.evaluate(()=>window.closeFixture.testReady());
  await testDialog().getByText('停止と音声の検証が完了しました',{exact:true}).waitFor();
  await testDialog().getByRole('button',{name:'マイクのテスト音声を再生',exact:true}).click();
  await testDialog().getByRole('button',{name:'一時停止',exact:true}).waitFor();
  await capture('test-dialog');await closeTest();
  await page.waitForFunction(()=>Object.keys(window.closeFixture.testPlayback.tokens).length===0&&Object.keys(window.closeFixture.testPlayback.reservations).length===0);
  check(await page.evaluate(()=>!window.closeFixture.captureTest),'closing retained test audio');
  check(await testButton().evaluate(e=>document.activeElement===e),'test close lost focus');
  await testButton().click();await consent.waitFor();await consent.getByRole('button',{name:'5秒テストを開始する',exact:true}).click();
  await page.evaluate(()=>window.closeFixture.testReady());await testDialog().getByRole('button',{name:'PCのテスト音声を再生',exact:true}).waitFor();
  check(await count('start_capture_test')===2,'new explicit test did not start once');
  await testDialog().getByRole('button',{name:'もう一度5秒録音して確認',exact:true}).click();
  await consent.waitFor();
  check(await page.locator('dialog[open]').count()===1&&await consent.evaluate(e=>e.classList.contains('capture-test-dialog')),'retest stacked a second dialog instead of using the existing boundary');
  await page.keyboard.press('Escape');await testDialog().waitFor();
  check(await count('start_capture_test')===2&&await testDialog().getByRole('button',{name:'PCのテスト音声を再生',exact:true}).isEnabled(),'declining retest lost results or started capture');
  await testDialog().getByRole('button',{name:'もう一度5秒録音して確認',exact:true}).click();
  await consent.waitFor();await consent.getByRole('button',{name:'5秒テストを開始する',exact:true}).click();
  await testDialog().getByRole('button',{name:'テストを取り消す',exact:true}).waitFor();
  check(await count('start_capture_test')===3,'explicit retest did not start exactly once');
  await testDialog().getByRole('button',{name:'テストを取り消す',exact:true}).click();
  await testDialog().getByText('5秒録音テストを終了しています。完了するまでお待ちください。',{exact:true}).waitFor();
  await page.evaluate(()=>window.closeFixture.testChanged({phase:'cancelled',busy:false}));
  await page.waitForFunction(()=>!window.closeFixture.captureTest);
  await closeTest();await testDialog().waitFor({state:'hidden'});
  results.push('test consent opens one dialog; closing releases playback and test audio and restores focus; retest uses the same dialog and cancellation automatically cleans up');
  await workspace().getByRole('button',{name:'録音を開始',exact:true}).click();
  const recordConsent=page.getByRole('dialog',{name:'選択した会議の録音を開始しますか？',exact:true});
  await recordConsent.getByRole('button',{name:'録音を開始する',exact:true}).click();
  await workspace().getByRole('button',{name:'録音を終了',exact:true}).waitFor();
  await page.evaluate(()=>{
    const f=window.closeFixture;
    f.refreshInputLevels=true;f.refreshWrites=true;f.refreshInputStatus=true;
    f.queueState={lanes:{microphone:[],system:[]},closed:{},loss:{microphone:0,system:0}};
    const owner=f.recording.sessionId;
    f.inputLevels={schemaVersion:1,ownerId:owner,revision:1,reportAgeMs:0,microphone:{state:'measured',windowFrames:1024,ageMs:0,rms:0.25,peak:0.5},system:{state:'measured',windowFrames:1024,ageMs:0,rms:0.125,peak:0.25}};
    f.writes={schemaVersion:1,ownerId:owner,revision:1,microphone:{state:'written',frames:480,ageMs:0},system:{state:'written',frames:480,ageMs:0}};
    f.inputStatus={schemaVersion:4,ownerId:owner,revision:1,reportAgeMs:0,microphone:{state:'connected',sourceEpoch:1,interruptions:0,reason:null},system:{state:'connected',sourceEpoch:1,interruptions:0,reason:null},notices:[],noticeLoss:{microphone:0,system:0}};
    return f.recordChanged({phase:'recording',elapsedMs:1122000});
  });
  await workspace().getByText('この端末に保存中',{exact:true}).waitFor();
  check(await page.getByRole('button',{name:'録音を終了',exact:true}).count()===1,'duplicate end action');
  check(await workspace().getByRole('button',{name:'録音を開始',exact:true}).count()===0,'disabled start retained in recording');
  check(await page.locator('.header-timer').count()===0&&await workspace().getByRole('meter').count()===2,'duplicate timer or missing meters');
  for(const [width,height] of [[1280,720],[1366,768]])for(const theme of ['light','dark']){
    await page.setViewportSize({width,height});await page.evaluate(t=>document.documentElement.dataset.theme=t,theme);
    await fits();await capture(`recording-${width}-${theme}`);
  }
  results.push('recording table, separate fresh evidence and single timer/end action fit both desktop sizes and themes');
  for(const width of [960,360]){
    await page.setViewportSize({width,height:640});await fits(false);
    await workspace().getByRole('button',{name:'録音を終了',exact:true}).scrollIntoViewIfNeeded();
    await capture(`recording-${width}`);
  }
  await page.setViewportSize({width:1280,height:720});
  await workspace().getByRole('button',{name:'録音を終了',exact:true}).click();
  check(await count('request_recording_confirmation')===1&&await count('stop_recording')===0,'end bypassed existing confirmation');
  results.push('narrow views retain all controls without horizontal overflow; end still requests native confirmation instead of stopping directly');
  await boot();await page.setViewportSize({width:360,height:640});
  await page.locator('#recording-title').fill('長い会議名'.repeat(30));
  await scope().selectOption('application');
  await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するアプリ"]')?.disabled);
  await fits(false);await capture('new-360-warning');
  results.push('long title and unselected-app warning remain accessible in narrow preparation');
  await page.setViewportSize({width:1280,height:720});
  for(const uncertain of [false,true]){
    await boot();
    await page.evaluate(uncertain=>{
      if(uncertain){window.closeFixture.testAbsentStart=true;window.closeFixture.testReconcileFailure=true;}
      else window.closeFixture.testLost=true;
    },uncertain);
    await testButton().click();await consent.getByRole('button',{name:'5秒テストを開始する',exact:true}).click();
    await page.getByRole('alert').filter({hasText:'処理を完了できませんでした'}).first().waitFor();
    await page.keyboard.press('Escape');
    await testDialog().waitFor();
    check(await page.locator('dialog[open]').count()===1&&await count('start_capture_test')===1,'failed receipt duplicated the test dialog or resent capture');
    check(await workspace().getByRole('button',{name:'録音を開始',exact:true}).isDisabled(),'unconfirmed capture enabled competing recording');
    if(uncertain){
      check(await testDialog().locator('.common-modal-body>button.secondary').isDisabled(),'unknown result allowed restart');
      await testDialog().getByRole('alert').filter({hasText:'開始要求の受付結果が未確認'}).waitFor();
    }else await testDialog().getByText('テストの入力を初期化しています',{exact:true}).waitFor();
  }
  results.push('lost and unknown start receipts still open one status dialog after failure, block competing capture and never resend start');
  await boot();await page.evaluate(()=>window.closeFixture.captureInputs.microphonePermission='denied');
  await testButton().click();
  await workspace().getByRole('status').filter({hasText:'選択したマイクまたは権限'}).waitFor();
  check(await page.locator('dialog[open]').count()===0&&await count('start_capture_test')===0,'preflight failure left an empty test dialog or started capture');
  results.push('failed input preflight reports the error without opening an empty modal or capturing');
  await boot();await page.evaluate(()=>window.closeFixture.failFlush=true);
  await testButton().click();await consent.getByRole('button',{name:'5秒テストを開始する',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'模擬編集保存失敗'}).first().waitFor();
  await page.keyboard.press('Escape');await consent.waitFor({state:'hidden'});
  check(await page.locator('dialog[open]').count()===0&&await count('start_capture_test')===0,'failed edit save opened an empty result dialog or captured');
  await testButton().click();await testDialog().waitFor();
  await testDialog().getByRole('alert').filter({hasText:'編集の保存を完了できませんでした'}).waitFor();
  await testDialog().getByRole('button',{name:'5秒録音して確認',exact:true}).click();await consent.waitFor();
  check(await count('start_capture_test')===0,'failed edit save prevented explicit consent retry');
  results.push('failed edit save returns to the initial trigger without an empty result dialog and permits a new explicit confirmation');
  return {passed:results.length,results};
}
