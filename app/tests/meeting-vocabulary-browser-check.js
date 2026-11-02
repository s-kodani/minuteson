// Production meeting editor and save barriers; synthetic IPC, no real audio.
async (page, capturePath) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  const dialog=()=>page.getByRole("dialog",{name:"会議の用語（任意）",exact:true});
  const trigger=()=>page.getByRole("button",{name:"会議の用語",exact:true});
  const term=()=>dialog().getByRole("textbox",{name:"用語（必須）",exact:true});
  const saved=()=>dialog().getByRole("status").filter({hasText:"用語は保存済み"}).waitFor();
  const close=async()=>{await page.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});};
  const open=async()=>{await trigger().click();await dialog().waitFor();};
  const clearNotices=()=>page.evaluate(async()=>{const {toasts}=await import('/src/toast.tsx');for(const notice of toasts.getSnapshot())toasts.dismiss(notice.key,notice.id);});
  const goMeeting=async title=>{
    await page.locator('.screen-navigation').getByRole("button",{name:"会議一覧",exact:true}).click();
    await page.getByRole("button",{name:title,exact:true}).click();
  };
  await page.evaluate(()=>{
    const f=window.monBrowserFixture;
    f.vocabulary["meeting-0"]={id:"meeting:meeting-0",lockVersion:1,payload:[{id:crypto.randomUUID(),term:"点検日",reading:"てんけんび",note:"取り込み時の補足",enabled:true}]};
  });
  await page.getByRole("button",{name:"会議A",exact:true}).click();await open();
  check(await term().inputValue()==="点検日","saved import vocabulary was not displayed");
  check(await dialog().getByRole("textbox",{name:"読み（任意）",exact:true}).inputValue()==="てんけんび","reading was lost");
  check(await dialog().getByRole("textbox",{name:"補足（任意・保存表示用）",exact:true}).inputValue()==="取り込み時の補足","note was lost");
  const enabled=dialog().getByRole("checkbox",{name:"有効",exact:true});
  await enabled.uncheck();await saved();
  check(await page.evaluate(()=>!window.monBrowserFixture.vocabulary["meeting-0"].payload[0].enabled),"disabled state did not persist");
  await enabled.check();
  await term().fill("点検予定日");await close();await open();
  check(await term().inputValue()==="点検予定日","closing discarded latest vocabulary");await saved();await close();
  check(await trigger().evaluate(e=>e===document.activeElement),"dictionary did not restore trigger focus");
  results.push("saved vocabulary fields load, edits survive closing, autosave completes and focus returns");

  await goMeeting("会議B");await open();
  check(await term().count()===0,"another meeting inherited vocabulary");
  await dialog().getByRole("button",{name:"用語を追加",exact:true}).click();await term().fill("会議B専用");await saved();await close();
  await goMeeting("会議A");await open();
  check(await term().inputValue()==="点検予定日","switching meetings lost or replaced saved vocabulary");
  results.push("meeting switch persists edits and keeps dictionaries isolated");

  for(const width of [960,360])for(const theme of ["light","dark"]){
    await page.setViewportSize({width,height:640});await page.evaluate(t=>document.documentElement.dataset.theme=t,theme);
    check(await dialog().evaluate(e=>e.scrollWidth<=e.clientWidth+1),"vocabulary dialog has horizontal overflow");
    await page.screenshot({path:capturePath(`meeting-vocabulary-${theme}-${width}.png`)});
  }
  await page.setViewportSize({width:1200,height:900});await close();
  results.push("dictionary dialog fits 960px and 360px in light and dark themes");

  await open();await page.evaluate(()=>{window.monBrowserFixture.failVocabulary=true;});
  await term().fill("保存失敗の入力");await close();
  const targetReads=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="get_transcription_regeneration_target").length);
  await page.getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  await page.getByRole("alert").filter({hasText:"編集の保存を完了できませんでした"}).waitFor();
  check(await page.evaluate(n=>window.monBrowserFixture.calls.filter(c=>c.command==="get_transcription_regeneration_target").length===n,targetReads),"failed vocabulary save allowed regeneration confirmation");
  check(await page.getByRole("dialog",{name:"文字起こしを再生成",exact:true}).count()===0,"save failure opened generation confirmation");
  await clearNotices();await open();
  check(await term().inputValue()==="保存失敗の入力","save failure discarded dictionary input");
  await page.evaluate(()=>{window.monBrowserFixture.failVocabulary=false;});
  await dialog().getByRole("button",{name:"用語保存を再試行",exact:true}).click();await saved();
  results.push("failed dictionary save blocks regeneration, preserves input and permits explicit retry");

  await page.evaluate(()=>{window.monBrowserFixture.persistenceDelayMs=500;});
  await term().fill("再生成で使う用語");await close();await clearNotices();
  await page.getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  const confirmation=page.getByRole("dialog",{name:"文字起こしを再生成",exact:true});await confirmation.waitFor();
  check(await page.evaluate(()=>window.monBrowserFixture.vocabulary["meeting-0"].payload[0].term==="再生成で使う用語"),"confirmation preceded the in-flight vocabulary save");
  await confirmation.getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow?.stage==="transcription");
  check(await page.evaluate(()=>window.monBrowserFixture.regenerationVocabulary[0].term==="再生成で使う用語"),"generation used stale vocabulary");
  await open();check(await term().inputValue()==="再生成で使う用語","generation remounted or reset dictionary");await close();
  results.push("immediate regeneration waits for saved vocabulary and retains the dictionary after admission");
  return {passed:results.length,results};
}
