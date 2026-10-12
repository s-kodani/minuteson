// Deterministic delayed IPC only; no user meetings, recordings or models.
async page => {
  const results=[],check=(value,message)=>{if(!value)throw Error(message);};
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture,invoke=window.__TAURI_INTERNALS__.invoke;
    f.seedRegeneration();
    window.__TAURI_INTERNALS__.invoke=(command,args)=>{
      if(command==="get_analysis_flow"&&f.holdOldRead){
        f.holdOldRead=false;
        return new Promise((_,reject)=>{f.releaseOldRead=()=>reject(Error("模擬遅延状態読込失敗"));});
      }
      if(command==="get_analysis_flow"&&f.failCurrentRead)throw Error("模擬現在の状態読込失敗");
      if(command==="get_transcription_regeneration_target"&&f.failAction){f.failAction=false;throw Error("模擬再生成操作失敗");}
      return invoke(command,args);
    };
    await f.emitNotification();
  });
  await page.getByRole("button",{name:"会議管理・録音・解析操作",exact:true}).click();
  const tools=page.getByRole("dialog",{name:"会議管理・録音・解析操作",exact:true});
  const regen=tools.getByRole("button",{name:"文字起こしを再生成",exact:true});
  await regen.waitFor();
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.holdOldRead=true;await f.emitNotification();});
  await page.waitForFunction(()=>!!window.monBrowserFixture.releaseOldRead);
  await regen.click();
  const confirmation=page.getByRole("dialog",{name:"文字起こしを再生成",exact:true});
  await confirmation.getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  await confirmation.waitFor({state:"hidden"});
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==="transcription");
  await page.evaluate(async()=>{const f=window.monBrowserFixture;await f.emitNotification();f.releaseOldRead();});
  await page.waitForFunction(()=>!document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
  await page.getByRole("status").filter({hasText:"文字起こし中"}).waitFor();
  await page.getByRole("alert").filter({hasText:"模擬遅延状態読込失敗"}).waitFor({state:"hidden"});
  check(await regen.isDisabled(),"running generation can be started twice");
  results.push("late read failure after a new action is resolved by a verified fresh read, not left as a persistent toast");

  await page.evaluate(()=>window.monBrowserFixture.finishRegeneration());
  await page.waitForFunction(()=>window.monBrowserFixture.currentId==="regenerated-analysis");
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==="review");
  await page.waitForFunction(()=>!document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.failCurrentRead=true;await f.emitNotification();});
  const current=page.getByRole("alert").filter({hasText:"模擬現在の状態読込失敗"});
  await current.waitFor();
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await current.isVisible(),"current read failure was suppressed");
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.failCurrentRead=false;await f.emitNotification();});
  await current.waitFor({state:"hidden"});
  results.push("current read failures remain visible and are resolved only after read recovery");

  await page.evaluate(()=>{window.monBrowserFixture.failAction=true;});
  await regen.click();
  const action=page.getByRole("alert").filter({hasText:"模擬再生成操作失敗"});
  await action.waitFor();
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await action.isVisible(),"successful polling hid a real action failure");
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="regenerate_transcription").length)===1,"error refresh sent a second generation");
  results.push("a real action failure survives successful polling; recovery does not resend generation");
  return {passed:results.length,results};
}
