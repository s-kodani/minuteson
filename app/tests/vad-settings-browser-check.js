(async function (page, capturePath, { approveModelLicense }) {
  const check=(v,m)=>{if(!v)throw Error(m);}, results=[];
  const model=page.getByRole('combobox',{name:'文字起こしのモデル',exact:true});
  const row=page.locator('.model-selection-row').filter({has:model});
  await model.waitFor();
  await model.selectOption('official:whisper-synthetic-medium');
  await row.getByRole('button',{name:'ダウンロードして使用',exact:true}).click();
  await approveModelLicense();
  const panel=page.getByRole('region',{name:'発話区間検知（VAD）',exact:true});
  await panel.waitFor();
  const toggle=panel.getByRole('checkbox',{name:'発話区間検知（VAD）',exact:true});
  const apply=panel.getByRole('button',{name:'VAD設定を適用',exact:true});
  const reset=panel.getByRole('button',{name:'標準値に戻す',exact:true});
  const field=label=>panel.getByRole('spinbutton',{name:label,exact:true});
  check(await toggle.isChecked(),'VAD not enabled by default');
  check(await panel.getByRole('button',{name:'VAD詳細設定',exact:true}).count()===0,'obsolete disclosure exists');
  check(await panel.getByRole('spinbutton').count()===6,'six options missing');
  check(await field('発話判定の閾値').inputValue()==='0.60','wrong default threshold');
  check(await field('最小発話時間（ms）').inputValue()==='300','wrong minimum speech default');
  check(await apply.isDisabled(),'unchanged settings can be applied');
  await field('発話判定の閾値').fill('');
  check(await apply.isDisabled(),'blank allowed');
  await page.getByRole('alert').waitFor();
  await reset.click();
  await field('最大発話区間（秒）').fill('1');
  await field('最小発話時間（ms）').fill('2000');
  check(await apply.isDisabled(),'contradictory durations accepted');
  await reset.click();
  await field('発話判定の閾値').fill('0.75');
  await field('最小発話時間（ms）').fill('450');
  await apply.click();
  await page.waitForFunction(()=>window.modelDownloadFixture.settingsByEngine['whisper.cpp'].vadOptions.thresholdMilli===750);
  check(await field('最小発話時間（ms）').inputValue()==='450','saved value lost');
  results.push('default-on, six fields, invalid values and explicit integer-valued save');
  const callsBefore=await page.evaluate(()=>window.modelDownloadFixture.calls.filter(c=>['start_model_download','check_model_compatibility'].includes(c.command)).length);
  await toggle.uncheck();
  check(await field('発話判定の閾値').isDisabled(),'off leaves numeric input active');
  await panel.getByRole('button',{name:'発話判定の閾値の説明を開く',exact:true}).click();
  const help=page.getByRole('dialog',{name:'発話判定の閾値',exact:true});await help.waitFor();
  check((await help.innerText()).includes('小声も除外されやすくなります'),'disabled field help is unavailable');
  check(await panel.locator('label .info-trigger').count()===0,'help button nested in input label');
  await page.keyboard.press('Escape');await help.waitFor({state:'hidden'});
  await apply.click();
  await page.waitForFunction(()=>window.modelDownloadFixture.settingsByEngine['whisper.cpp'].vadOptions.enabled===false);
  check(await field('発話判定の閾値').inputValue()==='0.75','turning off reset numeric options');
  await toggle.check(); await apply.click();
  await page.waitForFunction(()=>window.modelDownloadFixture.settingsByEngine['whisper.cpp'].vadOptions.enabled===true);
  await reset.click();
  check(await page.evaluate(()=>window.modelDownloadFixture.settingsByEngine['whisper.cpp'].vadOptions.thresholdMilli)===750,'reset saved automatically');
  await apply.click();
  await page.waitForFunction(()=>window.modelDownloadFixture.settingsByEngine['whisper.cpp'].vadOptions.thresholdMilli===600);
  check(await page.evaluate(()=>window.modelDownloadFixture.calls.filter(c=>['start_model_download','check_model_compatibility'].includes(c.command)).length)===callsBefore,'tuning downloaded or reran CPU');
  results.push('off keeps values, on restores them, reset waits for apply, tuning reuses the model');
  await model.selectOption('official:whisper-synthetic-large-v3');
  await row.getByRole('button',{name:'ダウンロードして使用',exact:true}).click();
  await approveModelLicense();
  await page.waitForFunction(()=>window.modelDownloadFixture.profiles[0].components[0].modelName==='Whisper large-v3（多言語）');
  await panel.waitFor();
  await page.evaluate(()=>window.modelDownloadFixture.setVisible(false)); await panel.waitFor({state:'hidden'});
  await page.evaluate(()=>window.modelDownloadFixture.setVisible(true)); await panel.waitFor();
  check(await toggle.isChecked(),'reopening or switching model lost enabled state');
  for(const width of [1280,390])for(const theme of ['light','dark']){
    await page.setViewportSize({width,height:1000});
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    const metrics=await panel.evaluate(el=>({
      gap:getComputedStyle(el.querySelector('.vad-fields')).gap,
      labels:[...el.querySelectorAll('.vad-fields>.info-field')].map(l=>({gap:getComputedStyle(l).gap,r:l.getBoundingClientRect().right})),
      inputs:[...el.querySelectorAll('input[type=number]')].map(i=>({h:i.getBoundingClientRect().height,font:getComputedStyle(i).fontSize,bg:getComputedStyle(i).backgroundColor})),
      buttons:[...el.querySelectorAll('.vad-actions button')].map(b=>b.getBoundingClientRect().height),
      actions:getComputedStyle(el.querySelector('.vad-actions')).gap,
      overflow:document.documentElement.scrollWidth>innerWidth
    }));
    check(metrics.gap==='16px'&&metrics.labels.every(l=>l.gap==='8px'&&l.r<=width),'field spacing differs from existing forms');
    check(metrics.inputs.every(i=>i.h===40&&i.font==='13px')&&metrics.buttons.every(h=>h>=40)&&metrics.actions==='12px','control metrics differ');
    check(!metrics.overflow,'form overflows viewport');
    await page.screenshot({path:`output/playwright/vad-settings-${width}-${theme}.png`,fullPage:true});
  }
  results.push('wide and narrow light/dark layouts follow 40px controls and 8/16/12px spacing');
  // An installed pre-VAD model matches the official display choice, but offers an upgrade.
  await page.evaluate(()=>{
    const f=window.modelDownloadFixture;
    const old=f.entries.find(e=>e.modelName==='Whisper medium（多言語）');
    old.modelFiles=old.modelFiles.filter(file=>file.role!=='vad'); old.download.recipeHash='d'.repeat(64);
    f.setVisible(false);
  });
  await panel.waitFor({state:'hidden'}); await page.evaluate(()=>window.modelDownloadFixture.setVisible(true)); await model.waitFor();
  await model.selectOption('official:whisper-synthetic-medium');
  await row.getByRole('button',{name:'VADを追加して使用',exact:true}).waitFor();
  results.push('installed pre-VAD weights offer an explicit VAD upgrade without duplicate choices');
  return {passed:results.length,results};
})
