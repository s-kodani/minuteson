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
  await mini.setViewportSize({width:360,height:132});await mini.goto("http://127.0.0.1:1420/?window=recorder");
  await modal.setViewportSize({width:560,height:460});await modal.goto("http://127.0.0.1:1420/?window=recording-confirmation");
  const state=()=>page.evaluate(()=>window.recordingFixture.read()), reset=async()=>{await page.evaluate(()=>window.recordingFixture.reset());await page.getByRole("button",{name:"録音を停止",exact:true}).waitFor();await page.waitForFunction(()=>document.querySelector('[aria-label="録音状態"] .stop-button')?.disabled===false);};
  const dialog=()=>modal.getByRole("dialog"), end=()=>modal.getByRole("button",{name:"録音を終了する",exact:true}), continued=()=>modal.getByRole("button",{name:"録音を続ける",exact:true});
  stage="decline";await reset();await mini.getByRole("button",{name:"停止",exact:true}).click();await dialog().waitFor();
  const original=(await state()).recording.sessionId;
  check(await mini.getByRole("dialog").count()===0&&await page.getByRole("dialog").count()===0,"confirmation was embedded in a recording window");
  await page.waitForFunction(()=>document.querySelector('[aria-label="録音状態"] .stop-button')?.disabled===true);check(await mini.getByRole("button",{name:"停止",exact:true}).isDisabled(),"mini stop remained enabled");
  check(await continued().evaluate(e=>e===document.activeElement)&&!(await state()).calls.some(c=>c.command==="synthetic-stop"),"unsafe initial focus or stopped before consent");
  await modal.keyboard.press("Enter");await dialog().waitFor({state:"hidden"});
  await mini.waitForFunction(()=>document.activeElement?.textContent==="停止");check((await state()).recording.sessionId===original&&(await state()).recording.phase==="recording","decline changed recording identity");
  results.push("mini uses a separate shared confirmation; initial Enter declines and returns exact opener while recording continues");

  stage="simultaneous";await reset();await Promise.all([page.evaluate(()=>window.__TAURI_INTERNALS__.invoke("request_recording_confirmation")),mini.evaluate(()=>window.__TAURI_INTERNALS__.invoke("request_recording_confirmation"))]);await dialog().waitFor();
  check((await state()).presentations===1,"simultaneous windows created duplicate confirmations");
  await end().dblclick();await modal.getByRole("status").filter({hasText:"停止・保存"}).waitFor();
  for(let i=0;i<5;i++)await modal.keyboard.press("Escape");
  check(await dialog().isVisible()&&(await state()).calls.filter(c=>c.command==="synthetic-stop").length===1,"accepted stop repeated or closed before saved");
  await page.evaluate(()=>window.recordingFixture.phase("saving"));check(await dialog().isVisible(),"saving was mistaken for complete");
  await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  await page.waitForFunction(()=>document.querySelector('.state-pill')?.textContent==="待機中");
  results.push("simultaneous requests share one decision; double-click stops once and repeated Escape waits for actual saving completion");

  await reset();await page.evaluate(()=>window.recordingFixture.configure({lostStop:true}));await page.getByRole("button",{name:"録音を停止",exact:true}).click();await end().click();
  await dialog().getByRole("alert").waitFor();check((await state()).calls.filter(c=>c.command==="synthetic-stop").length===1&&await end().isDisabled(),"lost response repeated stop");
  await modal.keyboard.press("Escape");check(await dialog().isVisible(),"uncertain stop dismissed");await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  results.push("lost accepted-stop acknowledgement retains same decision and never replays stopping");

  await reset();await page.getByRole("button",{name:"録音を停止",exact:true}).click();await end().click();await page.evaluate(()=>window.recordingFixture.finish(true));await dialog().getByRole("alert").filter({hasText:"保存失敗"}).waitFor();
  check(await modal.getByRole("heading",{name:"録音の保存状態を確認",exact:true}).isVisible()&&await end().isDisabled(),"save failure became successful completion");
  await modal.getByRole("button",{name:"確認を閉じる",exact:true}).click();await dialog().waitFor({state:"hidden"});check((await state()).recording.phase==="error","dismiss erased failed recording");
  results.push("failed saving remains failed with retained native state and no replay after dismissal");

  await reset();await page.evaluate(()=>window.recordingFixture.configure({suppressEvents:true}));await page.getByRole("button",{name:"録音を停止",exact:true}).click();await dialog().waitFor();await end().click();await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});await page.waitForFunction(()=>document.querySelector('.state-pill')?.textContent==="待機中");
  results.push("missed confirmation and session events recover by canonical periodic reads, not fabricated completion");

  await reset();await page.evaluate(()=>window.recordingFixture.phase("preparing"));await mini.getByRole("button",{name:"中止",exact:true}).click();await modal.getByRole("heading",{name:"録音準備を中止しますか？",exact:true}).waitFor();await page.evaluate(()=>window.recordingFixture.configure({transitionAtStop:true}));await modal.getByRole("button",{name:"録音準備を中止する",exact:true}).click();await dialog().getByRole("alert").waitFor();
  check(!(await state()).calls.some(c=>c.command==="synthetic-stop"),"outdated preparation consent stopped recording");await modal.getByRole("heading",{name:"録音を終了しますか？",exact:true}).waitFor();await end().click();check((await state()).calls.filter(c=>c.command==="synthetic-stop").length===1,"updated explicit consent repeated or failed stop");await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  results.push("preparation becoming recording invalidates old consent and requires a fresh explicit decision with updated loss warning");

  await reset();await page.getByRole("button",{name:"録音を停止",exact:true}).click();await dialog().waitFor();await page.evaluate(()=>window.recordingFixture.configure({failRead:true,suppressEvents:true}));await dialog().getByRole("alert").filter({hasText:"読戻し失敗"}).waitFor();check(await end().isDisabled(),"failed canonical read allowed stopping");await modal.keyboard.press("Escape");check(await dialog().isVisible(),"unknown read dismissed pending confirmation");await page.evaluate(()=>window.recordingFixture.configure({failRead:false}));await modal.waitForFunction(()=>!Array.from(document.querySelectorAll('button')).find(e=>e.textContent==="録音を終了する")?.disabled);await modal.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});check(!(await state()).calls.some(c=>c.command==="synthetic-stop"),"read recovery sent stop");
  results.push("lost canonical reads protect decisions; verified recovery clears read errors and declining never stops");

  await reset();await page.evaluate(()=>window.recordingFixture.configure({failStop:true}));await mini.getByRole("button",{name:"停止",exact:true}).click();await end().click();await dialog().getByRole("alert").filter({hasText:"模擬停止失敗"}).waitFor();const failedId=(await state()).decision.id;check(!(await state()).calls.some(c=>c.command==="synthetic-stop")&&await end().isDisabled(),"failed stop was replayed");await modal.getByRole("button",{name:"確認を閉じる",exact:true}).click();await dialog().waitFor({state:"hidden"});await page.evaluate(()=>window.recordingFixture.configure({failStop:false}));await mini.getByRole("button",{name:"停止",exact:true}).click();await dialog().waitFor();check((await state()).decision.id!==failedId&&await dialog().getByRole("alert").count()===0,"new explicit confirmation reused failed identity or stale error");await end().click();await page.evaluate(()=>window.recordingFixture.finish(false));await dialog().waitFor({state:"hidden"});
  results.push("failed stop stays visible without replay; only a new explicit confirmation can retry the same recording");

  await reset();await mini.getByRole("button",{name:"停止",exact:true}).click();await dialog().waitFor();await modal.setViewportSize({width:360,height:640});
  const box=await dialog().boundingBox();check(box.x>=23.9&&box.x+box.width<=336.1&&await dialog().evaluate(e=>e.scrollWidth<=e.clientWidth+1),"small separate confirmation clipped");await modal.screenshot({path:capturePath("recording-stop-confirmation-360.png")});await modal.setViewportSize({width:560,height:460});await modal.screenshot({path:capturePath("recording-stop-confirmation-560.png")});
  await modal.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});check(!(await state()).calls.some(c=>c.command==="synthetic-stop"),"visual check stopped recording");
  results.push("separate confirmation fits 560px and 360px; visual check and Escape never stop recording");

  stage="recording-quit";
  const requestQuit=()=>page.evaluate(()=>window.__TAURI_INTERNALS__.invoke("plugin:event|emit",{event:"main-close-requested",payload:{quit:true}}));
  const clearClose=()=>page.evaluate(()=>{window.dataFixture.calls=[];});
  const closed=()=>page.evaluate(()=>window.dataFixture.calls.filter(c=>c.command==="finish_main_close").length);
  const quitEnd=()=>modal.getByRole("button",{name:"録音を終了して閉じる",exact:true});
  await reset();await clearClose();await requestQuit();await modal.getByRole("heading",{name:"録音を終了してアプリを閉じますか？",exact:true}).waitFor();
  check((await state()).decision.quitAfterSave&&await page.getByRole("dialog").count()===0,"quit intent was missing or duplicated main confirmation");
  const quitBox=await dialog().boundingBox();check(quitBox.y>=23.9&&quitBox.y+quitBox.height<=436.1&&await dialog().evaluate(e=>e.scrollWidth<=e.clientWidth+1),"recording quit clipped the native-size confirmation");await modal.screenshot({path:capturePath("recording-quit-confirmation-560.png")});
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
  check(await mini.locator(".mini-error").textContent()===requestError&&JSON.stringify((await state()).recording)===beforeRequestError&&!(await state()).calls.some(c=>c.command==="synthetic-stop")&&await closed()===0,"native close request error changed recording or was lost");
  await mini.getByRole("button",{name:"停止",exact:true}).click();await dialog().waitFor();
  await page.getByRole("alert").filter({hasText:"合成確認ウィンドウ"}).waitFor({state:"hidden"});check(await mini.locator(".mini-error").count()===0,"new explicit confirmation retained stale request error");
  await modal.keyboard.press("Escape");await dialog().waitFor({state:"hidden"});
  results.push("native close request failure preserves canonical recording and reaches both recording UIs; a new explicit decision clears stale request error without stopping");
  return {passed:results.length,results};
 }catch(cause){throw Error(`${stage}: ${String(cause)}; modal=${await modal.locator("body").innerText()}; decision=${JSON.stringify((await page.evaluate(()=>window.recordingFixture.read())).decision)}`);}
 finally{await mini.close();await modal.close();await page.goto("about:blank");await context.unrouteAll();}
}
