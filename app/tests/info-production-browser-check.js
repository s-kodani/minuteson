// Current product settings; synthetic IPC, no native operations or recording.
async (page,capturePath)=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const info=label=>page.getByRole("button",{name:`${label}の説明を開く`,exact:true});
  const visit=async(label,text,details=false)=>{
    await info(label).click();
    const popup=page.getByRole("dialog",{name:label,exact:true});await popup.waitFor();
    if(details)await popup.getByRole("link",{name:"詳しい説明",exact:true}).click();
    check((await popup.innerText()).includes(text),`${label}: missing current explanation`);
    const bounds=await popup.boundingBox(),width=await page.evaluate(()=>document.documentElement.clientWidth);
    check(bounds.x>=7.9&&bounds.y>=7.9&&bounds.x+bounds.width<=width-7.9&&bounds.y+bounds.height<=await page.evaluate(()=>window.innerHeight)-7.9,"clipped help");
    if(details){
      check(await popup.evaluate(e=>e.scrollWidth<=e.clientWidth+1),"detailed help overflows horizontally");
      await popup.screenshot({path:capturePath("vocabulary-help-details.png")});
    }
    await page.keyboard.press("Escape");await popup.waitFor({state:"hidden"});
    check(await info(label).evaluate(e=>e===document.activeElement),`${label}: opener focus lost`);
  };
  await page.locator('.sidebar').getByRole("button",{name:"設定",exact:true}).click();
  await page.getByRole("heading",{name:"設定",exact:true}).waitFor();
  for(const [label,text] of [["表示テーマ","OSの明るさ"],["バックアップと追加復元","置き換えません"]]){
    await visit(label,text);results.push(`current ${label} help dismisses by keyboard and restores focus`);
  }
  await page.locator('.sidebar').getByRole("button",{name:"プロンプト",exact:true}).click();
  await page.setViewportSize({width:360,height:640});
  await visit("共通用語","次回",true);
  await page.setViewportSize({width:1200,height:900});
  await page.locator('.sidebar').getByRole("button",{name:"設定",exact:true}).click();
  for(const label of ["要確認データ","モデル本体削除","モデル配置記録","プロファイル構成記録"]){
    check(await info(label).count()===0,`${label}: retired maintenance help remains`);
  }
  results.push("retired maintenance/model-record help is absent from current settings");
  for(const theme of ["light","dark"]){
    await page.getByRole("combobox",{name:"表示モード",exact:true}).selectOption(theme);
    await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
    for(const width of [360,960]){
      await page.setViewportSize({width,height:640});await info("バックアップと追加復元").click();
      const popup=page.getByRole("dialog",{name:"バックアップと追加復元",exact:true});
      await popup.getByRole("link",{name:"詳しい説明",exact:true}).click();
      check((await popup.innerText()).includes("AIモデル本体"),"backup model exclusion missing");
      check(await page.evaluate(()=>document.documentElement.scrollWidth<=document.documentElement.clientWidth),"horizontal overflow");
      await page.screenshot({path:capturePath(`info-current-${theme}-${width}.png`)});
      await page.keyboard.press("Escape");await popup.waitFor({state:"hidden"});
    }
    results.push(`${theme}: current settings help fits 360px and supported 960px`);
  }
  check(await page.evaluate(()=>window.monBrowserFixture.calls.every(c=>! /^(start_|delete_|apply_|restore_|place_|retry_|save_(?!ui_preferences)|create_|confirm_)/u.test(c.command))),"help invoked a model/data/recording mutation");
  results.push("help causes no recording, model execution or data mutation");
  return {passed:results.length,results};
}
