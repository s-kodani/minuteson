// Deterministic delayed IPC only; no user meetings, recordings or models.
async (page, _capturePath, { applyPlaybackTheme }) => {
  const results=[],check=(value,message)=>{if(!value)throw Error(message);};
  await page.evaluate(()=>{
    const fixture=window.monBrowserFixture;
    fixture.seedRegeneration();
    fixture.analysisFlow.diarizationJob={state:"succeeded",outputAnalysisId:fixture.currentId};
    const rows=Array.from({length:45},(_,i)=>({id:`scroll-${i}`,trackId:"microphone",startUs:i*5_000_000,endUs:i*5_000_000+1_000_000,recognizedText:`発言 ${i}`,effectiveText:`発言 ${i}`,speakerIds:["speaker-a"],attributionState:"assigned"}));
    fixture.setUtterances(rows);
    const invoke=window.__TAURI_INTERNALS__.invoke;
    let held=false;
    window.__TAURI_INTERNALS__.invoke=(command,args)=>{
      if(command==="get_analysis_flow"&&!held){
        held=true;
        return new Promise(resolve=>{fixture.releaseInitialFlow=()=>{
          rows[0]={...rows[0],effectiveText:"読み直した発言"};
          fixture.setUtterances(rows);
          resolve(invoke(command,args));
        };});
      }
      return invoke(command,args);
    };
  });
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  const utterances=page.getByRole("region",{name:"発言本文",exact:true});
  await utterances.getByText("発言 44",{exact:true}).waitFor({state:"attached"});
  await page.waitForFunction(()=>!!window.monBrowserFixture.releaseInitialFlow);
  const before=await utterances.evaluate(el=>{el.scrollTop=360;return el.scrollTop;});
  check(before>=300,"utterance list does not have enough scroll range");
  await page.evaluate(()=>window.monBrowserFixture.releaseInitialFlow());
  await utterances.getByText("読み直した発言",{exact:true}).waitFor({state:"attached"});
  check(Math.abs(await utterances.evaluate(el=>el.scrollTop)-before)<=1,"reloading the analysis moved the utterance list to another scroll position");
  results.push("a delayed analysis refresh updates the transcript without resetting its scroll position");
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture,invoke=window.__TAURI_INTERNALS__.invoke;
    f.seedRegeneration();
    window.__TAURI_INTERNALS__.invoke=(command,args)=>{
      if(command==="get_analysis_flow"&&f.holdOldRead){
        f.holdOldRead=false;
        return new Promise((_,reject)=>{f.releaseOldRead=()=>reject(Error("模擬遅延状態読込失敗"));});
      }
      if(command==="get_analysis_flow"&&f.failCurrentRead)throw Error("模擬現在の状態読込失敗");
      if(command==="get_meeting"&&f.failMeetingRead)throw Error("模擬会議情報の再取得失敗");
      if(command==="get_transcription_regeneration_target"&&f.failAction){f.failAction=false;throw Error("模擬再生成操作失敗");}
      return invoke(command,args);
    };
    await f.emitNotification();
  });
  const regen=page.locator(".transcript-generation-host button").filter({hasText:"文字起こしを再生成"});
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
  await page.getByRole("alert").filter({hasText:"模擬遅延状態読込失敗"}).waitFor({state:"hidden"});
  check(await regen.isDisabled(),"running generation can be started twice");
  results.push("late read failure after a new action is resolved by a verified fresh read, not left as a persistent toast");

  await page.evaluate(()=>{
    const f=window.monBrowserFixture;
    f.failMeetingRead=true;
    return f.finishRegeneration();
  });
  await page.waitForFunction(()=>window.monBrowserFixture.currentId==="regenerated-analysis");
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==="review");
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(({command,args})=>command==="record_meeting_open_failure"&&args.step==="get_meeting"&&args.message.includes("模擬会議情報の再取得失敗")));
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.failMeetingRead=false;await f.emitNotification();});
  await page.waitForFunction(()=>!document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.failCurrentRead=true;await f.emitNotification();});
  const current=page.getByRole("status").filter({hasText:"文字起こしの状態を確認中です。"});
  await current.waitFor();
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(({command,args})=>command==="record_meeting_open_failure"&&args.step==="get_analysis_flow"&&args.message.includes("模擬現在の状態読込失敗")&&args.meetingId));
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await current.isVisible(),"current read failure was suppressed");
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(({command,args})=>command==="record_meeting_open_failure"&&args.step==="get_analysis_flow"&&args.message.includes("模擬現在の状態読込失敗")).length)===1,"unchanged polling failure was logged repeatedly");
  await page.getByRole("heading",{name:"発言一覧",exact:true}).waitFor();
  const panel=current.locator(".loading-overlay-panel");
  const heading=page.locator(".transcript-primary-heading");
  const title=heading.getByRole("heading",{name:"発言一覧",exact:true});
  const copy=heading.getByRole("button",{name:"文字起こしをコピー",exact:true});
  const jump=heading.getByRole("button",{name:"再生位置へ移動",exact:true});
  const vocabulary=heading.getByRole("button",{name:"会議の用語",exact:true});
  const originalViewport=page.viewportSize();
  for(const [theme,width,height] of [["dark",1280,820],["light",960,640]]){
    await page.setViewportSize({width,height});
    await applyPlaybackTheme(theme);
    const box=await panel.boundingBox();
    check(box&&Math.abs(box.x+box.width/2-width/2)<=1&&Math.abs(box.y+box.height/2-height/2)<=1,`loading panel is off center at ${width}px`);
    const titleBox=await title.boundingBox();
    let previousBox=titleBox;
    for(const button of [copy,jump,vocabulary,regen]){
      const actionBox=await button.boundingBox();
      check(actionBox&&actionBox.height===32&&Math.abs(actionBox.y+actionBox.height/2-titleBox.y-titleBox.height/2)<=1,`header action is misaligned at ${width}px`);
      check(actionBox.x>=previousBox.x+previousBox.width,`header actions are out of order at ${width}px`);
      previousBox=actionBox;
    }
    const backgrounds=await page.evaluate(()=>{
      const surface=element=>{
        for(let node=element;node;node=node.parentElement){
          const color=getComputedStyle(node).backgroundColor;
          if(color!=="rgba(0, 0, 0, 0)")return color;
        }
      };
      return [".transcript-primary-heading",".workspace-panel-side-content > .speaker-panel-body",".utterance-list"].map(selector=>surface(document.querySelector(selector)));
    });
    check(backgrounds[0]&&new Set(backgrounds).size===1,`transcript areas have different backgrounds in ${theme}`);
  }
  await page.setViewportSize(originalViewport);
  await page.emulateMedia({reducedMotion:"reduce"});
  check(await panel.locator(".loading-overlay-spinner").evaluate(node=>getComputedStyle(node).animationName)==="none","spinner ignores reduced motion");
  await page.emulateMedia({reducedMotion:null});
  await page.getByRole("tab",{name:"議事録",exact:true}).click();
  await page.getByRole("tabpanel",{name:"議事録",exact:true}).waitFor();
  await current.waitFor({state:"hidden"});
  await page.getByRole("tab",{name:"文字起こし",exact:true}).press("Enter");
  await current.waitFor();
  results.push("the centered panel follows the transcript tab, allows pointer and keyboard navigation, and respects reduced motion");
  results.push("transcript actions align and workspace surfaces match at desktop widths in dark and light themes");
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.failCurrentRead=false;await f.emitNotification();});
  await current.waitFor({state:"hidden"});
  results.push("read failures expose the current verification status until a successful read recovers it");

  await page.evaluate(()=>{window.monBrowserFixture.failAction=true;});
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
  await page.locator('.transcript-generation-host').getByRole('button',{name:'話者分離を再開',exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==='diarization');
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==='regenerate_transcription').length)===1,'handoff recovery reran transcription');
  check(await page.evaluate(()=>window.monBrowserFixture.analysisFlow.transcriptionJob.id)==='saved-transcription','handoff recovery discarded saved transcription');
  results.push('speaker handoff failures resume from the successful transcription without regenerating it');
  return {passed:results.length,results};
}
