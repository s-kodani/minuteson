// Production UI with synthetic IPC; never reads or changes a real meeting.
async page => {
  const check=(value,message)=>{if(!value)throw Error(message);};
  await page.route('**/index.html',route=>route.fulfill({contentType:'text/html',body:'<div id="root"></div><script type="module" src="/tests/browser-fixture.js"></script>'}));
  await page.goto('http://127.0.0.1:1420/index.html');
  await page.getByRole('button',{name:'設定',exact:true}).click();
  const region=page.getByRole('region',{name:'未検出発言の話者補完',exact:true});
  await region.waitFor();
  const similarity=region.getByRole('spinbutton',{name:'類似度の下限',exact:true});
  const margin=region.getByRole('spinbutton',{name:'次点との差の下限',exact:true});
  check(await region.getByRole('spinbutton').count()===2,'only review thresholds are configurable');
  const apply=region.getByRole('button',{name:'適用',exact:true});
  check(await similarity.inputValue()==='0.80','similarity default');
  check(await margin.inputValue()==='0.10','margin default');
  await similarity.fill('0.73');await margin.fill('0.06');await apply.click();
  await page.waitForFunction(()=>window.monBrowserFixture.speakerCompletionPreferences?.lockVersion===1);
  check(await apply.isDisabled(),'saved apply button');
  const payload=await page.evaluate(()=>window.monBrowserFixture.speakerCompletionPreferences.payload);
  check(JSON.stringify(payload)===JSON.stringify({similarityThresholdMilli:730,marginThresholdMilli:60}),'saved values');
  check(await page.getByRole('region',{name:'登録した声の自動適用',exact:true}).getByRole('spinbutton',{name:'類似度の下限',exact:true}).inputValue()==='0.80','independent voice settings');
  await similarity.fill('0.731');check(await apply.isDisabled(),'fractional similarity rejected');
  await similarity.fill('0.74');
  await page.evaluate(()=>{window.monBrowserFixture.loseCompletionPreferencesReply=true;});await apply.click();
  await region.getByRole('button',{name:'現在の状態を確認',exact:true}).waitFor();
  check(await similarity.inputValue()==='0.74','lost reply retains input');
  await region.getByRole('button',{name:'現在の状態を確認',exact:true}).click();
  check(await similarity.inputValue()==='0.74','readback retains saved similarity');check(await apply.isDisabled(),'readback saved');
  await region.scrollIntoViewIfNeeded();await region.screenshot({path:'output/playwright/speaker-body-only-settings.png'});
  await page.setViewportSize({width:390,height:844});await region.scrollIntoViewIfNeeded();
  check(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth),'narrow page overflow');
  await region.screenshot({path:'output/playwright/speaker-body-only-settings-narrow.png'});
  await page.setViewportSize({width:1280,height:1000});
  await page.evaluate(()=>{window.monBrowserFixture.jobs=[];});
  await page.getByRole('button',{name:'会議一覧',exact:true}).first().click();await page.getByRole('button',{name:'会議A',exact:true}).click();
  check(await page.getByRole('button',{name:'話者割り当てを更新',exact:true}).count()===0,'retired assignment update button remains');
  check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(c=>c.command==='update_speaker_assignments')),'opening a meeting starts a manual assignment update');
  return {settings:'passed',independentThresholds:true,invalidIncrements:'rejected',lostResponseReadback:'passed',narrowWidth:390,manualAssignmentUpdate:'removed'};
}
