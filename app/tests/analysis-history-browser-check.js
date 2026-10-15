// Synthetic IPC only; no recording or real model activity.
async page => {
  const check=(v,m)=>{if(!v)throw Error(m);};
  await page.route('**/index.html', route=>route.fulfill({contentType:'text/html',body:'<div id="root"></div><script type="module" src="/tests/browser-fixture.js"></script>'}));
  await page.goto('http://localhost:1420/index.html');
  await page.waitForFunction(()=>!!window.monBrowserFixture);
  const missing='文字起こしモデルが未導入です。設定のローカルAIでモデルを選び、ダウンロードして使用してください。保存済み音声は保持しています';
  await page.evaluate(message=>{const f=window.monBrowserFixture;f.seedRegeneration();f.analysisFlow={...f.analysisFlow,stage:'configuration',errorMessage:message};},missing);
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==='get_analysis_flow'));
  const notices=()=>page.getByRole('alert').filter({hasText:missing});
  check(await notices().count()===0,'opening saved failure showed a new toast');
  await page.getByRole('button',{name:'会議一覧',exact:true}).last().click();
  await page.evaluate(()=>{const f=window.monBrowserFixture;Object.assign(f.preferences.payload.standard,{executablePath:'/synthetic/whisper',modelPath:'/synthetic/model'});});
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  check(await notices().count()===0,'return after model installation replayed old failure');
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.analysisFlow={...f.analysisFlow,lockVersion:2,errorMessage:'新しい解析失敗'};await f.emitNotification();});
  await page.getByRole('alert').filter({hasText:'新しい解析失敗'}).waitFor();
  await page.getByRole('button',{name:'エラー通知を閉じる',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'新しい解析失敗'}).waitFor({state:'hidden'});
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await page.getByRole('alert').filter({hasText:'新しい解析失敗'}).count()===0,'polling repeated dismissed failure');
  await page.getByRole('button',{name:'会議一覧',exact:true}).last().click();
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  check(await page.getByRole('alert').filter({hasText:'新しい解析失敗'}).count()===0,'remount repeated dismissed failure');
  return {passed:5};
}
