// Synthetic IPC only. Neither real recordings nor model executions occur.
async page => {
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  const edit=()=>page.getByRole("button",{name:"00:00:00の発言を編集",exact:true});
  const editor=()=>page.getByRole("textbox",{name:"00:00:00の発言",exact:true});
  const regen=()=>page.locator(".transcript-generation-host button").filter({hasText:"文字起こしを再生成"});
  const dismissNotices=()=>page.evaluate(async()=>{const {toasts}=await import('/src/toast.tsx');for(const notice of toasts.getSnapshot())toasts.dismiss(notice.key,notice.id);});
  const start=async()=>{await dismissNotices();await regen().click();await page.getByRole("heading",{name:"文字起こしを再生成",exact:true}).waitFor();};
  await regen().waitFor();
  check(await page.getByRole("tabpanel",{name:"文字起こし",exact:true}).getByRole("button",{name:"文字起こしを生成",exact:true}).count()===0,"existing text was treated as an initial transcription");

  results.push("existing saved text can be regenerated without a prior automatic-flow record");
  await page.evaluate(async()=>{window.monBrowserFixture.seedRegeneration();await window.monBrowserFixture.emitNotification();});
  await start();
  await page.screenshot({path:"output/playwright/transcription-replacement-20261016/transcription-regeneration-confirmation.png"});
  await page.getByRole("button",{name:"確認をキャンセル",exact:true}).click();
  check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(c=>c.command==="regenerate_transcription")),"cancel started generation");
  results.push("replacement needs explicit confirmation; cancelling performs no generation");
  await page.evaluate(()=>{window.monBrowserFixture.regenerationMissingModels=true;});
  await start();await page.getByRole("dialog",{name:"文字起こしを再生成",exact:true}).getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  await page.getByText("設定のローカルAIでモデルを導入してください",{exact:false}).first().waitFor();

  check(await page.evaluate(()=>window.monBrowserFixture.currentId==="analysis"&&window.monBrowserFixture.analysisFlow.stage==="review"),"missing models changed head");
  results.push("missing models preserve current transcript and permit a new explicit attempt");
  await page.evaluate(()=>{window.monBrowserFixture.regenerationMissingModels=false;window.monBrowserFixture.preferences.payload.standard.threads=7;});
  await page.evaluate(()=>{
    const f=window.monBrowserFixture,invoke=window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke=(command,args)=>command==="regenerate_transcription"
      ? new Promise((resolve,reject)=>{f.releaseRegeneration=()=>{window.__TAURI_INTERNALS__.invoke=invoke;invoke(command,args).then(resolve,reject);};})
      : invoke(command,args);
  });
  await start();await page.getByRole("dialog",{name:"文字起こしを再生成",exact:true}).getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  await page.waitForFunction(()=>!!window.monBrowserFixture.releaseRegeneration);

  check(await page.locator("dialog[open]").count()===0,"slow admission kept a confirmation modal open");
  check(await page.locator("dialog[open]").count()===0,"closing left focus inside a confirmation modal");

  check(await regen().isDisabled(),"pending admission can be duplicated");

  await page.evaluate(()=>window.monBrowserFixture.releaseRegeneration());
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==="transcription");
  results.push("confirmation closes before delayed admission responds, restoring focus and guarding duplicate starts");

  check(await regen().isDisabled(),"running regeneration can be duplicated");
  check(await page.evaluate(()=>window.monBrowserFixture.regenerationGlobalThreads===7),"global settings ignored");

  await edit().click();await editor().fill("処理中に書いた未保存本文");
  await page.evaluate(()=>window.monBrowserFixture.finishRegeneration());
  const replacement=()=>page.getByRole("heading",{name:"生成結果に置き換えますか？",exact:true});
  await replacement().waitFor();
  check(await page.evaluate(()=>window.monBrowserFixture.draft.payload.utterances[0].effective_text==="処理中に書いた未保存本文"),"barrier omitted open field");
  check(await page.evaluate(()=>window.monBrowserFixture.currentId==="analysis"),"changed transcript replaced without confirmation");
  await page.screenshot({path:"output/playwright/transcription-replacement-20261016/transcription-replacement-confirmation.png"});
  await page.getByRole("button",{name:"確認をキャンセル",exact:true}).click();
  await replacement().waitFor({state:"hidden"});
  await page.waitForTimeout(3300);
  check(await replacement().count()===0,"cancelled confirmation reopened on polling");
  check(await edit().innerText()==="処理中に書いた未保存本文","cancellation lost human text");
  const starts=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="regenerate_transcription").length);

  await page.locator(".transcript-generation-host").getByRole("button",{name:"生成結果に置き換える",exact:true}).click();
  await replacement().waitFor();
  await page.getByRole("button",{name:"置き換える",exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.currentId==="regenerated-analysis");
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="regenerate_transcription").length)===starts,"replacement reran generation");

  await page.getByRole("button",{name:"00:00:00の発言を編集",exact:true}).filter({hasText:"再生成された本文"}).waitFor();
  await page.getByRole("alert").filter({hasText:"再生成の開始後"}).waitFor({state:"hidden"});
  check(await page.locator(".transcript-workspace-tools").count()===0,"candidate/options UI reintroduced");
  await page.screenshot({path:"output/playwright/transcription-replacement-20261016/transcription-regeneration-published.png"});
  results.push("confirmation publishes the completed result without regenerating, while cancellation retains edits and allows reopening");
  // A lost acknowledgement is proved by a read, without resending creation.
  await page.evaluate(()=>{window.monBrowserFixture.regenerationLostResponse=true;});
  const before=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="regenerate_transcription").length);
  await start();await page.getByRole("dialog",{name:"文字起こしを再生成",exact:true}).getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==="transcription");
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="regenerate_transcription").length)===before+1,"lost acknowledgement duplicated start");
  results.push("lost start response is read back with the same operation identity, without another generation");

  // Background publication must not depend on the target meeting being open.
  await page.getByRole("button",{name:"会議一覧",exact:true}).last().click();
  await page.getByRole("button",{name:"会議B",exact:true}).click();
  const publications=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="publish_regenerated_transcription").length);
  await page.evaluate(()=>window.monBrowserFixture.finishRegeneration());
  await page.waitForFunction(count=>window.monBrowserFixture.calls.filter(c=>c.command==="publish_regenerated_transcription").length===count+1,publications);
  check(await page.getByRole("heading",{name:"会議B",exact:true}).isVisible(),"background publication changed selected meeting");
  results.push("publication completes while another meeting is open, without changing the selected meeting");
  await page.getByRole("button",{name:"会議一覧",exact:true}).last().click();await page.getByRole("button",{name:"会議A",exact:true}).click();
  await page.evaluate(()=>{window.monBrowserFixture.regenerationLostResponse=true;window.monBrowserFixture.regenerationUnreadable=true;});
  const unknownBefore=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="regenerate_transcription").length);
  await start();await page.getByRole("dialog",{name:"文字起こしを再生成",exact:true}).getByRole("button",{name:"文字起こしを再生成",exact:true}).click();
  await page.getByRole("alert").filter({hasText:"模擬再生成受付応答喪失"}).waitFor();
  check(await page.getByRole("alert").filter({hasText:"模擬再生成受付応答喪失"}).count()===1,"admission failure was duplicated");

  await page.evaluate(()=>{window.monBrowserFixture.regenerationReadFail=false;});
  await dismissNotices();await regen().click();
  await page.waitForFunction(()=>!document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="regenerate_transcription").length)===unknownBefore+1,"unreadable acceptance minted or resent generation");
  results.push("unknown acceptance retains the original request; recovered readback proves acceptance without a second start");

  await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
  await page.evaluate(()=>{const f=window.monBrowserFixture;f.seedRegeneration();f.currentId=null;f.candidateVisible=false;f.draft=null;f.jobs=[];f.preferences.payload.standard.threads=6;f.analysisFlow={...f.analysisFlow,stage:"attention",errorMessage:"模擬の初回生成失敗",transcriptionJob:{id:"first-failed",kind:"transcription",state:"failed"}};f.regenerationLostResponse=true;});
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  await page.getByRole("tabpanel",{name:"文字起こし",exact:true}).getByRole("button",{name:"文字起こしを生成",exact:true}).click();
  await page.getByRole("dialog",{name:"文字起こしを生成",exact:true}).getByRole("button",{name:"文字起こしを生成",exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.analysisFlow.stage==="transcription");
  check(await page.evaluate(()=>{const f=window.monBrowserFixture,starts=f.calls.filter(c=>c.command==="regenerate_transcription");return starts.length===1&&starts[0].args.request.expected.target===null&&!f.analysisFlow.replacementRequested&&f.regenerationGlobalThreads===6&&!f.calls.some(c=>c.command==="retry_transcription");}),"failed first generation cannot start fresh or lost response duplicated admission");
  results.push("failed first transcription without saved text starts fresh using global settings, with lost-ack readback and no old-job retry");
  return {passed:results.length,results};
}
