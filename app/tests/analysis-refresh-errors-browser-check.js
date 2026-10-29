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
  const regen=page.locator(".transcript-generation-host button").filter({hasText:"文字起こしを再生成"});
  await regen.waitFor();
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.holdOldRead=true;await f.emitNotification();});
  await page.waitForFunction(()=>!!window.monBrowserFixture.releaseOldRead);
  if(await tools.isVisible())await tools.locator(':scope > .common-modal-dismiss-row > [data-dialog-cancel]').click();
  await regen.click();
  const confirmation=page.getByRole("dialog",{name:"文字起こしを再生成",exact:true});
  await confirmation.getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  await confirmation.waitFor({state:"hidden"});
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==="transcription");
  await page.evaluate(async()=>{const f=window.monBrowserFixture;await f.emitNotification();f.releaseOldRead();});
  await page.waitForFunction(()=>!document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
  await tools.waitFor({state:"hidden"});
  await page.getByRole("button",{name:"会議管理・録音・解析操作",exact:true}).click();
  await page.getByRole("status").filter({hasText:"文字起こしの開始を準備"}).waitFor();
  await page.getByRole("alert").filter({hasText:"模擬遅延状態読込失敗"}).waitFor({state:"hidden"});
  check(await regen.isDisabled(),"running generation can be started twice");
  results.push("late read failure after a new action is resolved by a verified fresh read, not left as a persistent toast");

  await page.evaluate(()=>window.monBrowserFixture.finishRegeneration());
  await page.waitForFunction(()=>window.monBrowserFixture.currentId==="regenerated-analysis");
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==="review");
  await page.waitForFunction(()=>!document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.failCurrentRead=true;await f.emitNotification();});
  const current=page.getByRole("status").filter({hasText:"文字起こしの状態を確認中です。"});
  await current.waitFor();
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await current.isVisible(),"current read failure was suppressed");
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.failCurrentRead=false;await f.emitNotification();});
  await current.waitFor({state:"hidden"});
  results.push("read failures expose the current verification status until a successful read recovers it");

  await page.evaluate(()=>{window.monBrowserFixture.failAction=true;});
  if(await tools.isVisible())await tools.locator(':scope > .common-modal-dismiss-row > [data-dialog-cancel]').click();
  await regen.click();
  const action=page.getByRole("alert").filter({hasText:"模擬再生成操作失敗"});
  await action.waitFor();
  await page.waitForFunction(()=>{
    const card=[...document.querySelectorAll('.toast-card')].find(e=>e.textContent.includes('模擬再生成操作失敗'));
    if(!card)return false;
    const r=card.getBoundingClientRect();
    return card.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));
  });
  await action.locator('xpath=ancestor::section[contains(@class,"toast-card")]').getByRole('button',{name:'エラー通知を閉じる',exact:true}).click();
  await action.waitFor({state:'hidden'});
  check(await page.getByRole('tabpanel',{name:'文字起こし',exact:true}).isVisible(),'dismissing error hid the transcript');
  results.push('action errors appear above the native modal and can be dismissed without closing it');
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await action.count()===0,"polling resurrected a dismissed action failure");
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="regenerate_transcription").length)===1,"error refresh sent a second generation");
  results.push("a real action failure survives successful polling; recovery does not resend generation");
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture,invoke=window.__TAURI_INTERNALS__.invoke;
    f.analysisFlow={...f.analysisFlow,stage:'attention',errorMessage:'模擬話者分離への引き渡し失敗',transcriptionJob:{id:'saved-transcription',state:'succeeded',outputAnalysisId:f.currentId},diarizationJob:null};
    window.__TAURI_INTERNALS__.invoke=async(command,args)=>{
      if(command==='continue_analysis_flow'){
        if(args.expectedLockVersion!==f.analysisFlow.lockVersion)throw Error('模擬再開対象不一致');
        f.calls.push({command,args});
        f.analysisFlow={...f.analysisFlow,stage:'diarization',errorMessage:null,lockVersion:f.analysisFlow.lockVersion+1};
        await f.emitNotification();return structuredClone(f.analysisFlow);
      }
      return invoke(command,args);
    };
    await f.emitNotification();
  });
  await page.getByRole('button',{name:'会議管理・録音・解析操作',exact:true}).click();
  await tools.getByRole('button',{name:'話者分離を再開',exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==='diarization');
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==='regenerate_transcription').length)===1,'handoff recovery reran transcription');
  check(await page.evaluate(()=>window.monBrowserFixture.analysisFlow.transcriptionJob.id)==='saved-transcription','handoff recovery discarded saved transcription');
  results.push('speaker handoff failures resume from the successful transcription without regenerating it');
  return {passed:results.length,results};
}
