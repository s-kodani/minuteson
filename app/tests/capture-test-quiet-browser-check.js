// Production UI with synthetic test ownership and no device capture.
async page => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();await route.fulfill({response,body:(await response.text()).replace('/src/bootstrap.tsx','/tests/application-close-browser-fixture.js')});
  });
  await page.goto('http://127.0.0.1:1420/');
  await page.getByRole('button',{name:'新しい録音',exact:true}).click();
  const panel=page.getByRole('dialog',{name:'録音前の入力確認',exact:true});
  const begin=async()=>{
    if(await panel.isVisible())await panel.getByRole('button',{name:'5秒録音して確認',exact:true}).click();
    else await page.locator('.capture-test-panel>button').click();
    await page.getByRole('dialog',{name:'5秒の録音前テストを開始しますか？',exact:true}).getByRole('button',{name:'5秒テストを開始する',exact:true}).click();
    await panel.waitFor();
  };
  const close=()=>panel.getByRole('button',{name:'確認を閉じる',exact:true});
  await begin();await panel.getByRole('button',{name:'テストを取り消す',exact:true}).click();
  await panel.getByText('5秒録音テストを終了しています。完了するまでお待ちください。',{exact:true}).waitFor();
  await page.evaluate(()=>window.closeFixture.testChanged({phase:'cancelled',busy:false,errorMessage:null}));
  await page.waitForFunction(()=>!window.closeFixture.captureTest);
  check(await panel.isVisible()&&await page.locator('.toast-card').count()===0,'cancellation hid its dialog or emitted a toast');
  results.push('normal cancellation releases audio quietly and keeps the dialog available for another test');
  await begin();await page.evaluate(()=>window.closeFixture.testReady());
  await panel.getByText('停止と音声の検証が完了しました',{exact:true}).waitFor();
  await page.evaluate(()=>window.closeFixture.testDiscardFailure=true);await close().click();
  await panel.getByRole('alert').filter({hasText:'「閉じる」'}).first().waitFor();
  check(await page.locator('.toast-card').count()===0&&await panel.isVisible(),'cleanup failure escaped its modal');
  await page.evaluate(()=>window.closeFixture.testDiscardFailure=false);await close().click();await panel.waitFor({state:'hidden'});
  results.push('cleanup failure stays inline and closing retries without a separate discard action');
  await begin();await page.evaluate(()=>window.closeFixture.testChanged({phase:'error',busy:false,cleanupPending:true,errorMessage:'模擬の停止失敗'}));
  await panel.getByRole('alert').filter({hasText:'テスト音声の自動削除が完了していません'}).waitFor();
  await close().click();await panel.waitFor({state:'hidden'});
  check(await page.evaluate(()=>!window.closeFixture.captureTest),'terminal error audio was retained after closing');
  results.push('closing also releases audio after a failed capture');
  return {passed:results.length,results};
}
