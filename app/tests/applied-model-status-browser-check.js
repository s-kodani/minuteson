// Production model settings, synthetic metadata only. No model downloads or processes.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  const field=label=>page.getByRole('combobox',{name:label+'のモデル',exact:true});
  const row=label=>page.locator('.model-selection-row').filter({has:field(label)});
  const calls=command=>page.evaluate(command=>window.modelDownloadFixture.calls.filter(c=>c.command===command).length,command);
  await page.goto('http://127.0.0.1:1420/tests/model-download-browser.html?official=1');
  await field('文字起こし').waitFor();
  const options=await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('get_official_model_options'));
  const selections=[['文字起こし',options.find(o=>o.id==='whisper-synthetic-tiny')],['話者分離',options.find(o=>o.review.manifest.engine==='sherpa-onnx')],['議事録生成',options.find(o=>o.review.manifest.engine==='llama.cpp')]];
  for(const [label,option] of selections){
    await field(label).selectOption('official:'+option.id);
    await row(label).getByRole('button',{name:'ダウンロードして使用',exact:true}).click();
    await row(label).getByRole('button',{name:'使用中',exact:true}).waitFor();
    check(await row(label).getByRole('button',{name:'使用中',exact:true}).isDisabled(),label+': current package is not shown as applied');
  }
  results.push('all three checked applied models show disabled 使用中');
  const before=await calls('start_model_download');
  await page.evaluate(()=>{
    const f=window.modelDownloadFixture;
    for(const component of f.profiles[0].components){
      const entry=f.entries.find(e=>e.packageId===component.packageId);
      entry.download.recipeHash='d'.repeat(64);
    }
    f.setVisible(false);
  });
  await field('文字起こし').waitFor({state:'hidden'});
  await page.evaluate(()=>window.modelDownloadFixture.setVisible(true));
  for(const [label,option] of selections){
    await row(label).getByRole('button',{name:'使用中',exact:true}).waitFor();
    check(await field(label).inputValue()==='official:'+option.id,label+': lost selected model on catalog drift');
    check(await row(label).getByRole('button',{name:'使用中',exact:true}).isDisabled(),label+': transport metadata enabled duplicate preparation');
  }
  check(await calls('start_model_download')===before,'reopening settings acquired model files');
  for(const theme of ['light','dark']){
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    await page.screenshot({path:`output/windows-qa-work/applied-model-status-${theme}.png`,fullPage:true});
  }
  results.push('shared-runtime recipe drift and settings remount preserve applied state without download/VAD actions or repeated work');
  const reloadSettings=async()=>{
    await page.evaluate(()=>window.modelDownloadFixture.setVisible(false));
    await field('文字起こし').waitFor({state:'hidden'});
    await page.evaluate(()=>window.modelDownloadFixture.setVisible(true));
    await field('文字起こし').waitFor();
  };
  await page.evaluate(()=>{
    const f=window.modelDownloadFixture,component=f.profiles[0].components.find(c=>c.engine==='whisper.cpp');
    f.savedVadFiles=structuredClone(f.entries.find(e=>e.packageId===component.packageId).modelFiles);
    component.settingsMatch=false;f.profiles[0].configurationComplete=false;
  });
  await reloadSettings();
  await row('文字起こし').getByRole('button',{name:'使用',exact:true}).waitFor();
  check(await row('文字起こし').getByRole('button',{name:'VADを追加して使用',exact:true}).count()===0,'present VAD mislabeled as missing');
  results.push('an unapplied unchanged package offers reuse, never a false missing-VAD download');
  await page.evaluate(()=>{
    const f=window.modelDownloadFixture,component=f.profiles[0].components.find(c=>c.engine==='whisper.cpp');
    const entry=f.entries.find(e=>e.packageId===component.packageId);
    entry.modelFiles=entry.modelFiles.filter(file=>file.role!=='vad');component.settingsMatch=true;f.profiles[0].configurationComplete=true;
  });
  await reloadSettings();
  await row('文字起こし').getByRole('button',{name:'VADを追加して使用',exact:true}).waitFor();
  check(await row('文字起こし').getByRole('button',{name:'VADを追加して使用',exact:true}).isEnabled(),'actual missing VAD upgrade is blocked');
  results.push('an actual missing VAD retains an explicit upgrade action');
  await page.evaluate(()=>{
    const f=window.modelDownloadFixture,component=f.profiles[0].components.find(c=>c.engine==='whisper.cpp');
    f.entries.find(e=>e.packageId===component.packageId).modelFiles=structuredClone(f.savedVadFiles);
    component.compatibilityCurrent=false;f.profiles[0].configurationComplete=false;
  });
  await reloadSettings();
  await row('文字起こし').getByRole('button',{name:'使用',exact:true}).waitFor();
  const appliedBefore=await calls('apply_model_profile_application');
  await row('文字起こし').getByRole('button',{name:'使用',exact:true}).click();
  await row('文字起こし').getByRole('button',{name:'使用中',exact:true}).waitFor();
  check(await calls('start_model_download')===before&&await calls('apply_model_profile_application')===appliedBefore+1,'explicit reuse did not reapply its checked package');
  results.push('stale profile compatibility is not marked in-use; explicit reuse reapplies the existing checked package');
  await field('文字起こし').selectOption('official:whisper-synthetic-base');
  await row('文字起こし').getByRole('button',{name:'ダウンロードして使用',exact:true}).waitFor();
  check(await calls('start_model_download')===before,'selecting an uninstalled model started work');
  results.push('another uninstalled selection remains a download choice without automatic execution');
  return {passed:results.length,results};
}
