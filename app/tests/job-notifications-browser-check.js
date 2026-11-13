// Current notification UI: Sonner for new events, history in the menu only.
async page => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  const host=()=>page.getByRole("complementary",{name:"通知",exact:true});
  const status=text=>host().getByRole("status").filter({hasText:text});
  const alert=text=>host().getByRole("alert").filter({hasText:text});
  // These checks exercise notification state and guarded navigation. WebKit
  // can move the Sonner stack during pointer entry; activate its visible
  // controls by keyboard. Dedicated toast UI tests retain pointer coverage.
  const activateNotice=async control=>{await control.waitFor({state:"visible"});await control.focus();await control.press("Enter");};
  const calls=command=>page.evaluate(command=>window.monBrowserFixture.calls.filter(c=>c.command===command).length,command);
  const boot=async()=>{await page.reload();await page.getByRole("button",{name:"通知一覧",exact:true}).waitFor();await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==="get_analysis_notification_jobs"));};
  const start=async(patch={})=>{await page.evaluate(async patch=>{const f=window.monBrowserFixture;f.notificationJobs=[f.makeNotificationJob(patch)];await f.emitNotification();},patch);await status("受け付けました").waitFor();};
  const update=async patch=>{const before=await calls("get_analysis_notification_jobs");await page.evaluate(async patch=>{const f=window.monBrowserFixture;Object.assign(f.notificationJobs[0],{updatedAt:Date.now()},patch);await f.emitNotification();},patch);await page.waitForFunction(n=>window.monBrowserFixture.calls.filter(c=>c.command==="get_analysis_notification_jobs").length>n,before);};
  const dismiss=async()=>{
    const cards=host().locator('[data-sonner-toast]:not([data-removed=true]) .toast-card');
    await cards.first().getByRole("button",{name:/通知を閉じる$/}).waitFor({state:"visible"});
    // Dismiss by stable identity: removing a card shifts nth-based locators,
    // while Sonner may retain the removed card during its exit animation.
    const ids=await cards.evaluateAll(items=>items.map(item=>item.getAttribute("data-toast-id")));
    for(const id of ids){
      const selector=`.toast-card[data-toast-id=${JSON.stringify(id)}]`;
      const active=host().locator(`[data-sonner-toast]:not([data-removed=true]) ${selector}`);
      if(await active.count()===0)continue;
      await activateNotice(active.getByRole("button",{name:/通知を閉じる$/}));
      await host().locator(selector).waitFor({state:"hidden"});
    }
    await cards.waitFor({state:"hidden"});
  };
  const noWork=async()=>check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(c=>/^(start_|retry_|resume_analysis|cancel_analysis|regenerate_|publish_|adopt_|restore_)/.test(c.command))),"notification executed work");
  const history=()=>page.getByRole("dialog",{name:"通知一覧",exact:true});
  const openHistory=async()=>{await page.getByRole("button",{name:"通知一覧",exact:true}).click();await history().waitFor();};
  const closeHistory=async()=>{await history().getByRole("button",{name:"通知一覧を閉じる",exact:true}).click();await history().waitFor({state:"hidden"});};

  await boot();
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.notificationJobs=[f.makeNotificationJob({state:"succeeded",createdAt:1,updatedAt:2,outputAnalysisId:"historical"})];await f.emitNotification({state:"succeeded",outputAnalysisId:"forged-event"});});
  check(await host().locator(".toast-card").count()===0&&await calls("get_analysis_notification_output")===0,"historical/event payload announced success");
  results.push("historical success and forged event payload never announce completion");

  await boot();await start();await dismiss();
  await page.getByRole("button",{name:"設定",exact:true}).click();
  await update({state:"succeeded",completedUnits:2,outputAnalysisId:"exact-output"});await status("処理が完了しました").waitFor();
  const completedCard=host().locator(".toast-card").filter({hasText:"処理が完了しました"});
  const savedId=await completedCard.getAttribute("data-toast-id");
  await page.evaluate(()=>window.monBrowserFixture.emitNotification({id:"foreign",state:"succeeded"}));
  check(await completedCard.getAttribute("data-toast-id")===savedId&&await calls("get_analysis_notification_output")===1,"completion was not verified exactly once");
  await activateNotice(completedCard.getByRole("button",{name:"会議を開く",exact:true}));
  await page.getByRole("heading",{name:"会議A",exact:true}).waitFor();await noWork();
  results.push("verified completion notifies once across navigation and opens only its own meeting");

  for(const mode of ["lost","foreign","hidden"]){
    await boot();await start();await dismiss();await page.evaluate(mode=>window.monBrowserFixture.notificationProofMode=mode,mode);
    await update({state:"succeeded",outputAnalysisId:"unverified"});await page.waitForTimeout(500);
    check(await status("処理が完了しました").count()===0,"unverified output announced completion");
    check(await alert("解析通知の読込・結果確認").count()===0,"automatic output verification notified");
    await openHistory();
    check(await history().locator(".toast-card").filter({hasText:"解析通知の読込・結果確認"}).count()===0,"automatic verification entered history");
    await closeHistory();
    await page.evaluate(async()=>{window.monBrowserFixture.notificationProofMode="valid";await window.monBrowserFixture.emitNotification();});
    await status("処理が完了しました").waitFor();await noWork();
    results.push(mode+" proof holds completion; recovered read verifies the exact output without diagnostic notification history");
  }

  await boot();await start();await dismiss();await update({state:"failed",errorCode:"engine_failed"});
  await alert("engine_failed").waitFor();
  await activateNotice(host().getByRole("button",{name:"エラー通知を閉じる",exact:true}));
  await page.getByRole("button",{name:"設定",exact:true}).click();await openHistory();
  check(await history().locator(".toast-card").filter({hasText:"engine_failed"}).count()===1,"failure history duplicated or disappeared");
  check(await page.getByRole("region",{name:"解析の通知と要対応状態",exact:true}).count()===0,"removed inline history returned");
  await closeHistory();await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  await alert("engine_failed").waitFor({state:"hidden"});
  check(await alert("engine_failed").count()===0,"dismissed event was renotified");await noWork();
  results.push("failure remains once in menu history after dismissal/navigation; repeated reads do not renotify");

  await boot();await start();await dismiss();await page.evaluate(()=>window.monBrowserFixture.notificationProofMode="delay");
  await update({state:"succeeded",outputAnalysisId:"delayed-output"});await page.waitForFunction(()=>!!window.monBrowserFixture.releaseNotificationProof);
  await page.evaluate(async()=>{const f=window.monBrowserFixture;await Promise.all(Array.from({length:8},()=>f.emitNotification({outputAnalysisId:"forged"})));});
  check(await calls("get_analysis_notification_output")===1&&await status("処理が完了しました").count()===0,"in-flight proof duplicated or completed early");
  await page.evaluate(()=>{const f=window.monBrowserFixture;f.notificationProofMode="valid";f.releaseNotificationProof();});
  await status("処理が完了しました").waitFor();check(await calls("get_analysis_notification_output")===1,"proof repeated after invalidations");await noWork();
  results.push("rapid invalidations coalesce around one pending exact-output proof");

  await boot();await start();await dismiss();await update({state:"pausing"});
  check(await status("一時停止しました").count()===0,"pause request treated as checkpoint");
  await update({state:"paused",pauseReason:"recording"});await status("録音・取り込み優先").waitFor();await dismiss();
  await update({state:"cancelled",pauseReason:null});await status("取り消しました。現在版は保持").waitFor();await noWork();
  results.push("pause request, completed checkpoint and cancellation remain distinct");

  for(const kind of ["diarization","minutes_generation"]){
    await boot();await start({kind,meetingId:"meeting-1",meetingTitle:"会議B"});await dismiss();
    await page.evaluate(kind=>window.monBrowserFixture.notificationPublication=kind==="minutes_generation"?"markdown":"committed",kind);
    await update({state:"succeeded",completedUnits:2,totalUnits:2,outputAnalysisId:kind==="diarization"?"diar-output":null});
    await status(kind==="diarization"?"話者分離の結果を保存しました":"議事録を保存しました").waitFor();await noWork();
    results.push(kind+" verifies its own meeting and output format");
  }

  await boot();await start({kind:"minutes_generation"});await dismiss();
  await page.evaluate(()=>{const f=window.monBrowserFixture;f.notificationPublication="markdown";f.notificationWarnings=[];});
  await update({state:"succeeded",completedUnits:2,totalUnits:2,outputAnalysisId:null});
  const success=host().locator(".toast-success").filter({hasText:"議事録を保存しました"});await success.waitFor();
  check(await host().locator(".toast-warning").filter({hasText:"書式や根拠リンク"}).count()===0,"format check warning survived");
  check(await host().locator(".toast-error").count()===0,"free format appears as an error");
  const noticeId=await success.getAttribute("data-toast-id");
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await success.getAttribute("data-toast-id")===noticeId,"success duplicated on refresh");
  await noWork();
  results.push("minutes with a clean receipt display the ordinary save notice");

  await boot();await start({kind:"minutes_generation"});await dismiss();
  await page.evaluate(()=>{const f=window.monBrowserFixture;f.notificationPublication="markdown";f.notificationWarnings=["一部の音声リンクは要確認です"];});
  await update({state:"succeeded",completedUnits:2,totalUnits:2,outputAnalysisId:null});
  const warning=host().locator(".toast-warning").filter({hasText:"議事録を保存しました。一部の音声リンクは要確認です"});
  await warning.waitFor();
  check(await host().locator(".toast-success").filter({hasText:"議事録を保存しました"}).count()===0,"duplicate success notice");
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await warning.count()===1&&await calls("get_analysis_notification_output")===1,"citation warning duplicated on refresh");
  results.push("verified minutes with citation issues display one warning and no duplicate success notice");

  await boot();await page.getByRole("button",{name:"会議A",exact:true}).click();
  await page.getByRole("button",{name:"00:00:00の発言を編集",exact:true}).click();
  const field=page.getByRole("textbox",{name:"00:00:00の発言",exact:true});await field.fill("通知で失われない本文");
  await page.evaluate(()=>window.monBrowserFixture.failSave=true);await start({meetingId:"meeting-1",meetingTitle:"会議B"});
  await activateNotice(host().getByRole("button",{name:"会議を開く",exact:true}));
  // Saving and guarded navigation can publish two errors and replace the
  // front card. Wait for the exact result, then use the notification shortcut
  // to expand/read the stack without hovering a front card that is changing.
  await host().locator(".toast-card").filter({hasText:"編集内容はこの画面に保持"}).waitFor({state:"attached"});
  await page.keyboard.press("Alt+t");
  await alert("編集内容はこの画面に保持").waitFor();
  check(await field.inputValue()==="通知で失われない本文"&&await page.getByRole("heading",{name:"会議A",exact:true}).isVisible(),"notification navigation discarded unsaved input");await noWork();
  results.push("notification navigation respects failed edit saving and retains original meeting/input");

  await boot();await page.evaluate(()=>window.monBrowserFixture.emitWorkerError("模擬ワーカー停止"));
  await alert("模擬ワーカー停止").waitFor();
  check(!(await alert("模擬ワーカー停止").innerText()).includes("再起動後"),"ordinary generation error acquired a restart recommendation");
  await activateNotice(host().getByRole("button",{name:"エラー通知を閉じる",exact:true}));
  await page.getByRole("button",{name:"設定",exact:true}).click();await openHistory();
  check(await history().locator(".toast-card").filter({hasText:"模擬ワーカー停止"}).count()===1,"worker failure history lost");
  await closeHistory();await noWork();
  results.push("worker failure remains in menu history without automatic recovery/execution");

  await boot();await page.evaluate(()=>window.monBrowserFixture.emitWorkerError("ワーカー停止。再起動後にジョブ状態を確認してください。"));
  await alert("再起動後にジョブ状態を確認してください。").waitFor();await noWork();
  results.push("explicit native restart guidance is preserved without adding generic guidance");

  await boot();await page.evaluate(async()=>{const f=window.monBrowserFixture;f.notificationJobs=Array.from({length:55},(_,i)=>f.makeNotificationJob({meetingId:"historical-"+i,meetingTitle:"履歴会議"+i,createdAt:1,updatedAt:2,state:"failed",errorCode:"engine-"+i}));await f.emitNotification();});
  await page.waitForFunction(()=>window.monBrowserFixture.notifications().length===0);
  await openHistory();await page.waitForFunction(()=>document.querySelectorAll(".notifications-popover:popover-open [data-history-id]").length===55);
  check(await history().locator("[data-history-id]").count()===55,"historical failures not available in history");
  await history().getByRole("button",{name:"エラー通知を削除",exact:true}).first().click();
  await page.waitForFunction(()=>document.querySelectorAll(".notifications-popover:popover-open [data-history-id]").length===54);
  await page.evaluate(()=>window.monBrowserFixture.emitNotification());
  check(await history().locator("[data-history-id]").count()===54,"deleted history returned on read");await noWork();
  results.push("55 historical failures stay quiet and accessible; menu deletion does not reappear on refresh");
  return {passed:results.length,results};
}
