async (page, capturePath = path => path)=>{
 const context=page.context(), results=[], check=(ok,message)=>{if(!ok)throw Error(message);};
 let sequence=0;
 const id=()=>"00000000-0000-4000-8000-"+(++sequence).toString().padStart(12,"0");
 const empty=()=>({revision:0,id:null,phase:"idle",sessionId:null,meetingId:null,meetingTitle:null,origin:null,preparing:false,quitAfterSave:false,errorMessage:null});
 const initial=()=>({recording:{revision:1,phase:"recording",elapsedMs:45000,microphoneConnected:true,systemAudioConnected:true,meetingId:"synthetic-meeting",sessionId:id(),errorMessage:null},decision:empty(),calls:[],presentations:0,quitNotified:false,lostStop:false,failStop:false,failRead:false,suppressEvents:false});
 let backend=initial();
 const command=(name,args,label)=>{
  const s=backend,d=s.decision;
  if(name==="fixture-read")return s;
  if(name==="fixture-reset"){const r=s.recording.revision+1,v=d.revision+1;backend=initial();backend.recording.revision=r;backend.decision.revision=v;return null;}
  if(name==="fixture-configure"){Object.assign(s,args);return null;}
  if(name==="fixture-phase"){s.recording.phase=args.phase;s.recording.revision++;if(args.phase==="recording"&&d.phase==="confirming"){d.preparing=false;d.revision++;}return null;}
  if(name==="fixture-finish"){s.recording.phase=args.failed?"error":"idle";s.recording.revision++;s.recording.errorMessage=args.failed?"合成録音の保存失敗。元ファイルを保持しています":null;if(!args.failed){s.recording.sessionId=null;s.recording.meetingId=null;}d.phase=args.failed?"failed":"completed";d.errorMessage=s.recording.errorMessage;d.revision++;return null;}
  if(name==="get_recording_snapshot")return s.recording;
  if(name==="get_recording_confirmation"){if(s.failRead)throw Error("模擬確認読戻し失敗");return d;}
  s.calls.push({command:name,args,label});
  if(name==="hide_recorder_panel"||name==="open_main_window"){check(label==="recorder","invalid mini window action");return null;}
  if(name==="request_recording_confirmation"){if(["confirming","stopping","failed"].includes(d.phase))return d;check(["main","recorder"].includes(label)&&["preparing","recording"].includes(s.recording.phase)&&(!args.quit||label==="main"),"invalid recording request");s.decision={...empty(),revision:d.revision+1,id:id(),phase:"confirming",sessionId:s.recording.sessionId,meetingId:s.recording.meetingId,meetingTitle:"合成会議",origin:label,preparing:s.recording.phase==="preparing",quitAfterSave:!!args.quit};s.presentations++;return s.decision;}
  if(name==="decide_recording_confirmation"){
   check(label==="recording-confirmation"&&args.id===d.id&&["confirming","failed"].includes(d.phase),"確認は既に確定しています");if(!args.stop){d.phase="cancelled";d.revision++;return d;}
   if(s.transitionAtStop){s.transitionAtStop=false;s.recording.phase="recording";s.recording.revision++;d.preparing=false;d.revision++;}
   check(d.phase==="confirming"&&d.sessionId===s.recording.sessionId,"停止対象が変更されています");check(args.expectedPreparing===(s.recording.phase==="preparing"),"準備の状態が変更されています");
   d.phase=s.failStop?"failed":"stopping";d.errorMessage=s.failStop?"模擬停止失敗。録音状態を確認してください":null;d.revision++;if(!s.failStop){s.calls.push({command:"synthetic-stop",sessionId:d.sessionId});s.recording.phase="stopping";s.recording.revision++;}if(s.lostStop){s.lostStop=false;throw Error("模擬停止受付の応答喪失");}return d;
  }
  throw Error("unexpected synthetic recording command");
 };
 // One Node controller is authoritative. Per-renderer localStorage caches are
 // not a transaction store and must not fabricate equal-version contradictions.
 await context.route("http://127.0.0.1:1420/__synthetic_recording_controller",async route=>{
  const {command:name,args={},label}=route.request().postDataJSON();let value,error;
  try{value=structuredClone(command(name,args,label));}catch(cause){error=String(cause);}
  const notifyQuit=backend.decision.quitAfterSave&&backend.decision.phase==="completed"&&!backend.quitNotified;
  if(notifyQuit)backend.quitNotified=true;
  const frozen=structuredClone(backend);
  if(!name.startsWith("get_")&&name!=="fixture-read"&&!frozen.suppressEvents){await Promise.all(context.pages().map(p=>p.evaluate(async s=>{if(!window.recordingFixture)return;await window.__TAURI_INTERNALS__.invoke("plugin:event|emit",{event:"recording-confirmation-changed",payload:s.decision});await window.__TAURI_INTERNALS__.invoke("plugin:event|emit",{event:"session-changed",payload:s.recording});},frozen).catch(()=>{})));}
  if(notifyQuit)await page.evaluate(id=>window.__TAURI_INTERNALS__.invoke("plugin:event|emit",{event:"main-close-requested",payload:{quit:true,afterRecording:id}}),frozen.decision.id);
  await route.fulfill({contentType:"application/json",body:JSON.stringify(error?{error}:{value})});
 });
 await context.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{const response=await route.fetch();const body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/recording-confirmation-browser-fixture.js");</script>');await route.fulfill({response,body});});
 await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.recordingFixture);
 const mini=await context.newPage(), modal=await context.newPage();
 let stage="setup";
 try{
  await mini.setViewportSize({width:360,height:116});await mini.goto("http://127.0.0.1:1420/?window=recorder");
  await modal.setViewportSize({width:420,height:280});await modal.goto("http://127.0.0.1:1420/?window=recording-confirmation");
  // Use the always-visible production header. The maintenance modal's stop
  // button is intentionally hidden until a user opens that modal.
  const mainStop=()=>page.getByRole("button",{name:"録音を終了",exact:true});
  const state=()=>page.evaluate(()=>window.recordingFixture.read()), reset=async()=>{await page.evaluate(()=>window.recordingFixture.reset());await mainStop().waitFor();await page.waitForFunction(()=>document.querySelector('[aria-label="共通録音状態"] .stop-button')?.disabled===false);};
  const dialog=()=>modal.getByRole("dialog"), end=()=>modal.getByRole("button",{name:"録音を終了する",exact:true}), continued=()=>modal.getByRole("button",{name:"録音を続ける",exact:true});
  stage="decline";await reset();const miniStop=mini.getByRole("button",{name:"停止",exact:true}), miniOpen=mini.getByRole("button",{name:"メインを開く",exact:true});
  check(await miniStop.evaluate(e=>{const style=getComputedStyle(e);return style.borderWidth==="0px"&&style.backgroundColor==="rgba(0, 0, 0, 0)"&&!!e.querySelector("svg rect")}),"mini stop is not a borderless stop icon");
  check(await miniOpen.evaluate(e=>{const style=getComputedStyle(e);return style.borderWidth==="0px"&&style.backgroundColor==="rgba(0, 0, 0, 0)"}),"mini main action still has a frame");
  await miniOpen.locator("svg").click();check((await state()).calls.some(c=>c.command==="open_main_window"&&c.label==="recorder"),"clicking the main icon did not open main");
  await miniStop.locator("svg").click();await dialog().waitFor();
  const miniRequest=(await state()).calls.find(c=>c.command==="request_recording_confirmation"&&c.label==="recorder");
  check(miniRequest?.args?.anchor?.width>0&&miniRequest?.args?.anchor?.height>0,"mini stop icon did not pass its popup anchor");
  check(await dialog().getAttribute("aria-modal")==="false"&&await modal.locator(".common-modal").count()===0,"stop confirmation still uses a modal");
  check(await modal.getByRole("button",{name:"状態を再確認",exact:true}).count()===0,"manual status refresh remains in stop popup");
  const original=(await state()).recording.sessionId;
  check(await mini.getByRole("dialog").count()===0&&await page.getByRole("dialog").count()===0,"confirmation was embedded in a recording window");
  await page.waitForFunction(()=>document.querySelector('[aria-label="共通録音状態"] .stop-button')?.disabled===true);check(await mini.getByRole("button",{name:"停止",exact:true}).isDisabled(),"mini stop remained enabled");
  check(await continued().evaluate(e=>e===document.activeElement)&&!(await state()).calls.some(c=>c.command==="synthetic-stop"),"unsafe initial focus or stopped before consent");
  await modal.keyboard.press("Enter");await dialog().waitFor({state:"hidden"});
  await mini.waitForFunction(()=>document.activeElement?.getAttribute("aria-label")==="停止");check((await state()).recording.sessionId===original&&(await state()).recording.phase==="recording","decline changed recording identity");
  results.push("mini uses a separate shared confirmation; initial Enter declines and returns exact opener while recording continues");

  stage="simultaneous";await reset();await Promise.all([page.evaluate(()=>window.__TAURI_INTERNALS__.invoke("request_recording_confirmation")),mini.evaluate(()=>window.__TAURI_INTERNALS__.invoke("request_recording_confirmation"))]);await dialog().waitFor();
  check((await state()).presentations===1,"simultaneous windows created duplicate confirmations");
  await end().dblclick();await modal.getByRole("status").filter({hasText:"停止・保存"}).waitFor();
  for(let i=0;i<5;i++)await modal.keyboard.press("Escape");
  check(await dialog().isVisible()&&(await state()).calls.filter(c=>c.command==="synthetic-stop").length===1,"accepted stop repeated or closed before saved");
  await page.evaluate(()=>window.recordingFixture.phase("saving"));check(await dialog().isVisible(),"saving was mistaken for complete");
  await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  await page.getByLabel("共通録音状態",{exact:true}).getByText("待機中",{exact:true}).waitFor();
  results.push("simultaneous requests share one decision; double-click stops once and repeated Escape waits for actual saving completion");

  await reset();await page.evaluate(()=>window.recordingFixture.configure({lostStop:true}));await mainStop().click();check((await state()).calls.some(c=>c.command==="request_recording_confirmation"&&c.label==="main"&&c.args.anchor?.width>0),"main stop did not pass its popup anchor");await end().click();
  await dialog().getByRole("alert").waitFor();check((await state()).calls.filter(c=>c.command==="synthetic-stop").length===1&&await end().isDisabled(),"lost response repeated stop");
  await modal.keyboard.press("Escape");check(await dialog().isVisible(),"uncertain stop dismissed");await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  results.push("lost accepted-stop acknowledgement retains same decision and never replays stopping");

  await reset();await mainStop().click();await end().click();await page.evaluate(()=>window.recordingFixture.finish(true));await dialog().getByRole("alert").filter({hasText:"保存失敗"}).waitFor();
  check(await modal.getByRole("heading",{name:"録音の保存状態を確認",exact:true}).isVisible()&&await end().isDisabled(),"save failure became successful completion");
  await modal.getByRole("button",{name:"確認を閉じる",exact:true}).click();await dialog().waitFor({state:"hidden"});check((await state()).recording.phase==="error","dismiss erased failed recording");
  results.push("failed saving remains failed with retained native state and no replay after dismissal");

  await reset();await page.evaluate(()=>window.recordingFixture.configure({suppressEvents:true}));await mainStop().click();await dialog().waitFor();await end().click();await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});await page.getByLabel("共通録音状態",{exact:true}).getByText("待機中",{exact:true}).waitFor();
  results.push("missed confirmation and session events recover by canonical periodic reads, not fabricated completion");

  await reset();await page.evaluate(()=>window.recordingFixture.phase("preparing"));await mini.getByRole("button",{name:"中止",exact:true}).click();await modal.getByRole("heading",{name:"録音準備を中止しますか？",exact:true}).waitFor();await page.evaluate(()=>window.recordingFixture.configure({transitionAtStop:true}));await modal.getByRole("button",{name:"録音準備を中止する",exact:true}).click();await dialog().getByRole("alert").waitFor();
  check(!(await state()).calls.some(c=>c.command==="synthetic-stop"),"outdated preparation consent stopped recording");await modal.getByRole("heading",{name:"録音を終了しますか？",exact:true}).waitFor();await end().click();check((await state()).calls.filter(c=>c.command==="synthetic-stop").length===1,"updated explicit consent repeated or failed stop");await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  results.push("preparation becoming recording invalidates old consent and requires a fresh explicit decision with updated loss warning");

  await reset();await mainStop().click();await dialog().waitFor();await page.evaluate(()=>window.recordingFixture.configure({failRead:true,suppressEvents:true}));await dialog().getByRole("alert").filter({hasText:"読戻し失敗"}).waitFor();check(await end().isDisabled(),"failed canonical read allowed stopping");await modal.keyboard.press("Escape");check(await dialog().isVisible(),"unknown read dismissed pending confirmation");await page.evaluate(()=>window.recordingFixture.configure({failRead:false}));await modal.waitForFunction(()=>document.querySelector('.analysis-actions .stop-button')?.disabled===false);await modal.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});check(!(await state()).calls.some(c=>c.command==="synthetic-stop"),"read recovery sent stop");
  results.push("lost canonical reads protect decisions; verified recovery clears read errors and declining never stops");

  await reset();await page.evaluate(()=>window.recordingFixture.configure({failStop:true}));await mini.getByRole("button",{name:"停止",exact:true}).click();await end().click();await dialog().getByRole("alert").filter({hasText:"模擬停止失敗"}).waitFor();const failedId=(await state()).decision.id;check(!(await state()).calls.some(c=>c.command==="synthetic-stop")&&await end().isDisabled(),"failed stop was replayed");await modal.getByRole("button",{name:"確認を閉じる",exact:true}).click();await dialog().waitFor({state:"hidden"});await page.evaluate(()=>window.recordingFixture.configure({failStop:false}));await mini.getByRole("button",{name:"停止",exact:true}).click();await dialog().waitFor();check((await state()).decision.id!==failedId&&await dialog().getByRole("alert").count()===0,"new explicit confirmation reused failed identity or stale error");await end().click();await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  results.push("failed stop stays visible without replay; only a new explicit confirmation can retry the same recording");

  await reset();await mini.getByRole("button",{name:"停止",exact:true}).click();await dialog().waitFor();await modal.setViewportSize({width:360,height:280});await page.waitForTimeout(150);
  const box=await dialog().boundingBox();check(box.x>=0&&box.x+box.width<=360.1&&await dialog().evaluate(e=>e.scrollWidth<=e.clientWidth+1&&e.scrollHeight<=e.clientHeight+1),"small stop popup clipped");await modal.screenshot({path:capturePath("recording-stop-popup-360.png")});await modal.setViewportSize({width:420,height:280});await page.waitForTimeout(150);await modal.screenshot({path:capturePath("recording-stop-popup-420.png")});
  await modal.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});check(!(await state()).calls.some(c=>c.command==="synthetic-stop"),"visual check stopped recording");
  results.push("non-modal stop popup fits 420px and 360px; visual check and Escape never stop recording");

  stage="recording-quit";
  const requestQuit=()=>page.evaluate(()=>window.__TAURI_INTERNALS__.invoke("plugin:event|emit",{event:"main-close-requested",payload:{quit:true}}));
  const clearClose=()=>page.evaluate(()=>{window.dataFixture.calls=[];});
  const closed=()=>page.evaluate(()=>window.dataFixture.calls.filter(c=>c.command==="finish_main_close").length);
  const quitEnd=()=>modal.getByRole("button",{name:"録音を終了して閉じる",exact:true});
  await reset();await clearClose();await requestQuit();await modal.getByRole("heading",{name:"録音を終了してアプリを閉じますか？",exact:true}).waitFor();
  check((await state()).decision.quitAfterSave&&await page.getByRole("dialog").count()===0,"quit intent was missing or duplicated main confirmation");
  const quitBox=await dialog().boundingBox();check(quitBox.y>=0&&quitBox.y+quitBox.height<=280.1&&await dialog().evaluate(e=>e.scrollWidth<=e.clientWidth+1&&e.scrollHeight<=e.clientHeight+1),"recording quit clipped the native-size popup");await modal.screenshot({path:capturePath("recording-quit-popup-420.png")});
  await modal.keyboard.press("Enter");await dialog().waitFor({state:"hidden"});check(await closed()===0&&!(await state()).calls.some(c=>c.command==="synthetic-stop"),"declining quit stopped or closed recording");
  results.push("recording quit shares the independent confirmation and declining never stops or closes the app");

  await reset();await clearClose();await Promise.all([requestQuit(),requestQuit()]);await quitEnd().waitFor();await quitEnd().dblclick();
  await modal.getByRole("status").filter({hasText:"停止・保存"}).waitFor();for(let i=0;i<3;i++)await modal.keyboard.press("Escape");
  check(await closed()===0&&(await state()).calls.filter(c=>c.command==="synthetic-stop").length===1,"quit repeated stopping or exited before saved");
  await page.evaluate(()=>window.recordingFixture.phase("saving"));check(await closed()===0,"quit exited during saving");await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});await page.waitForFunction(()=>window.dataFixture.calls.filter(c=>c.command==="finish_main_close").length===1);
  await page.evaluate(()=>window.recordingFixture.finish(false));check(await closed()===1,"completion repeated application closing");
  check(!await page.evaluate(()=>window.dataFixture.calls.some(c=>c.command==="stop_recording")),"quit bypassed shared recording stop");
  results.push("recording quit stops once, waits for actual save and continues closing exactly once with the completed decision");

  await reset();await clearClose();await requestQuit();await quitEnd().click();await page.evaluate(()=>window.recordingFixture.finish(true));await dialog().getByRole("alert").filter({hasText:"保存失敗"}).waitFor();check(await closed()===0,"failed recording save continued application closing");await modal.getByRole("button",{name:"確認を閉じる",exact:true}).click();await dialog().waitFor({state:"hidden"});check(await closed()===0&&(await state()).recording.phase==="error","closing failed confirmation erased save failure or quit");
  results.push("recording quit retains saving failure and never continues closing after dismissal");
  stage="native-close-request-error";await reset();await clearClose();
  const beforeRequestError=JSON.stringify((await state()).recording);
  const requestError="録音状態と保存結果を保持しています。新しい停止は送っていません: 合成確認ウィンドウの表示失敗";
  await Promise.all([page,mini].map(p=>p.evaluate(message=>window.__TAURI_INTERNALS__.invoke("plugin:event|emit",{event:"recording-confirmation-request-error",payload:message}),requestError)));
  await page.getByRole("alert").filter({hasText:"合成確認ウィンドウ"}).waitFor();
  await mini.getByRole("alert").filter({hasText:"合成確認ウィンドウ"}).waitFor();
  check((await mini.getByRole("alert").filter({hasText:"合成確認ウィンドウ"}).innerText()).includes(requestError)&&await mini.getByRole("button",{name:"録音パネルの説明を開く",exact:true}).getAttribute("data-help-error")==="true"&&await mini.locator(".mini-error").count()===0&&JSON.stringify((await state()).recording)===beforeRequestError&&!(await state()).calls.some(c=>c.command==="synthetic-stop")&&await closed()===0,"native close request error changed recording or was lost");
  await mini.getByRole("button",{name:"停止",exact:true}).click();await dialog().waitFor();
  await page.getByRole("alert").filter({hasText:"合成確認ウィンドウ"}).waitFor({state:"hidden"});await mini.getByRole("alert").filter({hasText:"合成確認ウィンドウ"}).waitFor({state:"hidden"});await mini.waitForFunction(()=>document.querySelector(".recorder-info-trigger button")?.dataset.helpError==="false");check(await mini.getByRole("button",{name:"録音パネルの説明を開く",exact:true}).getAttribute("data-help-error")==="false","new explicit confirmation retained stale request error");
  await modal.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});
  results.push("native close request failure preserves canonical recording and reaches both recording UIs; a new explicit decision clears stale request error without stopping");

  // Hiding a panel is independent of a shared stop decision. Exercise the
  // real close button while the other renderer owns the confirmation.
  const hideWithoutMutation=async()=>{
   const before=await state(), close=mini.getByRole("button",{name:"ミニパネルを閉じる",exact:true});
   check(await close.isEnabled(),"pending confirmation disabled mini close");
   await close.click();
   await mini.waitForFunction(()=>document.querySelector('[aria-label="ミニパネルを閉じる"]')?.disabled===false);
   const after=await state(), added=after.calls.slice(before.calls.length);
   check(JSON.stringify(after.recording)===JSON.stringify(before.recording)&&JSON.stringify(after.decision)===JSON.stringify(before.decision),"mini close changed canonical recording or decision");
   check(added.length===1&&added[0].command==="hide_recorder_panel"&&added[0].label==="recorder","mini close requested stop or application exit");
  };
  stage="hide-confirming";await reset();await mini.getByRole("button",{name:"停止",exact:true}).click();await dialog().waitFor();
  await hideWithoutMutation();check(await dialog().isVisible(),"hiding mini dismissed separate confirmation");
  await modal.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});
  check((await state()).recording.phase==="recording"&&!(await state()).calls.some(c=>c.command==="synthetic-stop"),"decline after hiding stopped recording");
  results.push("mini close stays enabled during confirmation, hides exactly once and declining still preserves recording");

  stage="hide-stopping-saving";await reset();await mainStop().click();await end().click();
  await modal.getByRole("status").filter({hasText:"停止・保存"}).waitFor();await hideWithoutMutation();
  await mini.reload();await mini.getByRole("button",{name:"ミニパネルを閉じる",exact:true}).waitFor();
  check(await mini.getByRole("button",{name:"ミニパネルを閉じる",exact:true}).isEnabled()&&await dialog().isVisible(),"remount lost independent close or pending decision");
  await page.evaluate(()=>window.recordingFixture.phase("saving"));await hideWithoutMutation();
  check(await dialog().isVisible()&&(await state()).calls.filter(c=>c.command==="synthetic-stop").length===1,"hide/remount replayed stop or completed saving");
  await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  results.push("mini hide and remount during stopping/saving preserve one accepted stop and wait for canonical completion");

  stage="hide-unreadable-decision";await reset();await mainStop().click();await dialog().waitFor();
  await page.evaluate(()=>window.recordingFixture.configure({failRead:true,suppressEvents:true}));
  await dialog().getByRole("alert").filter({hasText:"読戻し失敗"}).waitFor();
  await mini.waitForFunction(()=>document.querySelector('.mini-bottom .mini-stop')?.disabled===true);
  await hideWithoutMutation();check(await dialog().isVisible()&&await end().isDisabled(),"mini hide relaxed unreadable stop guard");
  await page.evaluate(()=>window.recordingFixture.configure({failRead:false}));
  await modal.waitForFunction(()=>document.querySelector('.analysis-actions .stop-button')?.disabled===false);
  await modal.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});
  check(!(await state()).calls.some(c=>c.command==="synthetic-stop"),"read recovery or hide stopped recording");
  results.push("canonical decision read failure blocks stopping but not mini close; recovery and decline do not send stop");

  stage="hide-failed-save";await reset();await mainStop().click();await end().click();
  await page.evaluate(()=>window.recordingFixture.finish(true));await dialog().getByRole("alert").filter({hasText:"保存失敗"}).waitFor();
  await hideWithoutMutation();check(await dialog().isVisible()&&(await state()).decision.phase==="failed","mini hide erased save failure");
  await modal.getByRole("button",{name:"確認を閉じる",exact:true}).click();await dialog().waitFor({state:"hidden"});
  check((await state()).recording.phase==="error"&&(await state()).calls.filter(c=>c.command==="synthetic-stop").length===1,"failed save dismissal retried or declared success");
  results.push("mini close remains independent after failed saving and preserves error without replaying stop");
  return {passed:results.length,results};
 }catch(cause){throw Error(`${stage}: ${String(cause)}; mini=${await mini.locator("body").innerText()}; modal=${await modal.locator("body").innerText()}; decision=${JSON.stringify((await page.evaluate(()=>window.recordingFixture.read())).decision)}`);}
 finally{await mini.close();await modal.close();await page.goto("about:blank");await context.unrouteAll();}
}
