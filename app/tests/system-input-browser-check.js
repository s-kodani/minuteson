// Product main/mini UI with synthetic binding reports only. Never captures audio.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');await route.fulfill({response,body});
  });
  const input=kind=>page.locator(`[data-input-kind="${kind}"]`);
  const states=async(microphone,system)=>page.waitForFunction(([m,s])=>document.querySelector('[data-input-kind="microphone"]')?.dataset.inputState===m&&document.querySelector('[data-input-kind="system"]')?.dataset.inputState===s,[microphone,system]);
  const binding=(state="connected",sourceEpoch=1,interruptions=0,reason=null)=>({state,sourceEpoch,interruptions,reason});
  const set=async(microphone,system,targets={})=>page.evaluate(([microphone,system,targets])=>{
    const f=window.closeFixture,id=f.recording.sessionId,notices=[];
    for(const input of ["microphone","system"]){const m=input==="microphone"?microphone:system,target=targets[input];if(!m||!target)continue;const kind=m.state==="connected"?"recovered":"interrupted";notices.push({id:`${id}:${input}:${m.interruptions}:${kind}`,input,target,kind,interruptions:m.interruptions});}
    f.inputStatus={schemaVersion:2,ownerId:id,revision:(f.inputStatus?.revision??0)+1,reportAgeMs:0,microphone,system,notices};
  },[microphone,system,targets]);
  const toast=kind=>page.locator(".toast-card").filter({hasText:kind==="system"?/PC音声の取得|PC音声取得/u:/マイクの取得/u});
  const count=()=>page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_input_status").length);
  const begin=async mini=>{
    await page.goto(`http://127.0.0.1:1420/${mini?"?window=recorder":""}`);await page.waitForFunction(()=>!!window.closeFixture);
    if(!mini){await page.getByRole("button",{name:"新しい録音",exact:true}).click();await page.getByRole("button",{name:"録音を開始",exact:true}).click();const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});}
    await page.evaluate(mini=>{const f=window.closeFixture;f.recordChanged({phase:"recording",...(mini?{sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID()}:{}),microphoneConnected:true,systemAudioConnected:true});f.refreshInputStatus=true;},mini);
  };
  try{
    await begin(false);await states("unknown","unknown");results.push("neither legacy connection flag substitutes for an authoritative binding report");
    await set(binding(),binding());await states("connected","connected");check(await toast("system").count()===0,"initial PC connection claimed recovery");results.push("initial PC binding does not claim recovery or write success");
    await set(binding(),binding("disconnected",1,1,"stream_stopped"),{system:"main"});await states("connected","disconnected");await toast("system").waitFor();
    check(await toast("microphone").count()===0,"PC interruption changed microphone notification");check((await input("system").innerText()).includes("取得処理が停止"),"PC reason missing");results.push("PC stream stop has its own reason and leaves microphone connected");
    await toast("system").getByRole("button",{name:"注意通知を閉じる",exact:true}).click();await page.waitForTimeout(650);check(await toast("system").count()===0&&await input("system").getAttribute("data-input-state")==="disconnected","dismissed PC warning repeated or cleared retained state");results.push("PC warning dismissal preserves connection issue without heartbeat repetition");
    await set(binding(),binding("unavailable",1,1,"permission_unavailable"),{system:"main"});await states("connected","unavailable");check((await input("system").innerText()).includes("PC音声の権限"),"PC permission was mislabeled microphone");check(await toast("system").count()===0,"same episode repeated");results.push("PC permission and reason changes remain explicit without duplicating an episode");
    await set(binding(),binding("unavailable",1,1,"device_disconnected"));await states("unknown","unknown");check(await toast("system").count()===0,"invalid lane reason retained notification");
    await set(binding(),binding("unavailable",1,1,"application_unavailable"),{system:"main"});await states("connected","unavailable");check(await toast("system").count()===0,"invalid report erased prior dismissal");results.push("wrong-lane reasons invalidate the pair and later valid reads do not redeliver dismissal");
    await set(binding(),binding("reconnecting",2,1,"acquisition_timeout"),{system:"main"});await states("connected","reconnecting");check((await input("system").innerText()).includes("応答待ち"),"PC timeout reason missing");results.push("PC acquisition timeout is distinct from microphone disconnect and stays pending");
    await set(binding(),binding("connected",2,1),{system:"main"});await states("connected","connected");await toast("system").waitFor();check((await toast("system").innerText()).includes("同じ対象"),"PC recovery did not preserve selected scope");results.push("confirmed new PC epoch reports recovery of the same scope");
    await toast("system").getByRole("button",{name:"情報通知を閉じる",exact:true}).click();
    await set(binding("disconnected",1,1,"device_disconnected"),binding("disconnected",2,2,"stream_stopped"),{microphone:"main",system:"main"});await states("disconnected","disconnected");await toast("system").waitFor();await toast("microphone").waitFor();
    check(await toast("system").count()===1&&await toast("microphone").count()===1,"simultaneous notices collided");results.push("simultaneous interruptions have independent main notifications");
    await toast("system").getByRole("button",{name:"注意通知を閉じる",exact:true}).click();check(await toast("microphone").count()===1,"closing PC warning dismissed microphone warning");results.push("closing one main input notification does not dismiss the other");
    await toast("microphone").getByRole("button",{name:"注意通知を閉じる",exact:true}).click();
    for(const [width,theme] of [[1280,"light"],[960,"dark"],[360,"dark"]]){
      await page.setViewportSize({width,height:900});await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);await input("system").scrollIntoViewIfNeeded();
      check(await input("system").evaluate(e=>e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth+1),`PC state overflow ${width}`);await page.screenshot({path:`both-input-main-${width}-${theme}.png`});results.push(`both retained input states fit ${width}px ${theme}`);
    }
    await page.getByRole("button",{name:"設定",exact:true}).click();await page.getByRole("button",{name:"録音画面へ",exact:true}).click();await states("disconnected","disconnected");check(await toast("system").count()===0&&await toast("microphone").count()===0,"workspace remount repeated input notices");results.push("both input dismissal records survive workspace remount");
    await page.evaluate(()=>window.closeFixture.refreshInputStatus=false);await states("stale","stale");check(await toast("system").count()===0,"stale PC report retained current notice");results.push("a frozen paired publication expires both states without a saving claim");
    await page.evaluate(()=>window.closeFixture.refreshInputStatus=true);await set(binding("disconnected",1,1,"device_disconnected"),binding("disconnected",2,2,"stream_stopped"),{microphone:"main",system:"main"});await states("disconnected","disconnected");check(await toast("system").count()===0&&await toast("microphone").count()===0,"fresh version repeated dismissed events");results.push("a new heartbeat restores states but not dismissed notices");
    await set(binding("disconnected",1,1,"device_disconnected"),binding("stopped",2,2));await states("disconnected","stopped");
    await set(binding("disconnected",1,1,"device_disconnected"),binding("connected",3,2));await states("unknown","unknown");results.push("late PC activation cannot undo a confirmed stopped binding");
    await page.evaluate(()=>{const f=window.closeFixture;f.inputStatus=null;f.recordChanged({sessionId:crypto.randomUUID()});});await set(binding(),null);await states("connected","unknown");check((await input("system").innerText()).includes("確認できません"),"unsupported backend invented connection");results.push("unsupported PC binding is explicit unknown while microphone can be connected");
    await page.evaluate(()=>{window.closeFixture.inputStatus.schemaVersion=1;});await states("unknown","unknown");results.push("old single-input schema cannot masquerade as paired binding evidence");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"stopping"}));await states("unknown","unknown");const stopped=await count();await page.waitForTimeout(650);check(await count()===stopped,"stopping kept polling both states");results.push("stopping removes both states and ceases the shared poll");
    await begin(true);await page.setViewportSize({width:360,height:116});
    await set(binding("disconnected",1,1,"device_disconnected"),binding("disconnected",1,1,"stream_stopped"),{microphone:"main",system:"recorder"});await states("disconnected","disconnected");await page.locator(".mini-input-notice").waitFor();
    check((await page.locator(".mini-input-notice").innerText()).includes("PC音声中断")&&!(await page.locator(".mini-input-notice").innerText()).includes("マイク中断"),"mini duplicated main-routed mic warning");results.push("mini displays only the PC event routed to it and retains both input issues");
    await set(binding("disconnected",2,2,"device_disconnected"),binding("disconnected",1,1,"stream_stopped"),{microphone:"recorder",system:"recorder"});await page.getByRole("button",{name:"入力通知を閉じる",exact:true}).waitFor();
    check((await page.locator(".mini-input-notice").innerText()).includes("マイク中断 / PC音声中断"),"mini simultaneous warning lost a track");
    check(await page.locator(".mini-shell").evaluate(e=>e.scrollHeight<=e.clientHeight+1&&e.scrollWidth<=e.clientWidth+1),"both-input mini overflowed native 360x116 window");
    check(await input("microphone").evaluate(e=>e.clientWidth>=75)&&await input("system").evaluate(e=>e.clientWidth>=65),"mini status labels were squeezed out");await page.screenshot({path:"both-input-mini-interrupted-360x116.png"});results.push("simultaneous warnings and both labels fit native mini geometry");
    await page.getByRole("button",{name:"入力通知を閉じる",exact:true}).click();await page.waitForTimeout(650);await states("disconnected","disconnected");check(await page.locator(".mini-input-notice").count()===0,"mini aggregate dismissal repeated");results.push("mini aggregate dismissal retains both issues without repetition");
    await set(binding("disconnected",2,2,"device_disconnected"),binding("connected",2,1),{microphone:"recorder",system:"recorder"});await states("disconnected","connected");await page.locator(".mini-input-notice").waitFor();check((await page.locator(".mini-input-notice").innerText()).includes("PC音声復帰"),"independent PC recovery missing");results.push("PC-only recovery leaves microphone interruption explicit in mini");
    await set(binding("connected",3,2),binding("connected",2,1),{microphone:"recorder",system:"recorder"});await states("connected","connected");await page.getByRole("button",{name:"入力通知を閉じる",exact:true}).waitFor();check((await page.locator(".mini-input-notice").innerText()).includes("マイク復帰 / PC音声復帰"),"mini paired recovery missing");
    check(await page.locator(".mini-shell").evaluate(e=>e.scrollHeight<=e.clientHeight+1&&e.scrollWidth<=e.clientWidth+1),"mini paired recovery overflow");await page.screenshot({path:"both-input-mini-recovered-360x116.png"});results.push("paired verified recovery fits mini and never claims persisted PCM");
    await page.evaluate(()=>window.closeFixture.failInputStatus=true);await states("unknown","unknown");await page.evaluate(()=>window.closeFixture.failInputStatus=false);await states("connected","connected");check(await page.locator(".mini-input-notice").count()===0,"mini reread redelivered recovery");results.push("failed paired reads clear statuses without later redelivering recovery");
    check(await page.evaluate(()=>window.closeFixture.calls.every(c=>!["start_capture_test","start_recording","stop_recording"].includes(c.command))),"synthetic test unexpectedly issued native capture command");results.push("mini verification does not start or stop native capture");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`);}finally{await page.context().unrouteAll();}
}
