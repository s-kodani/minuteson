// Current live-input telemetry through the production provider and synthetic IPC.
// No native capture, microphone, or PC audio is accessed.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const states=(mic,system)=>page.waitForFunction(([m,s])=>document.querySelector('[data-input-kind="microphone"]')?.dataset.inputState===m&&document.querySelector('[data-input-kind="system"]')?.dataset.inputState===s,[mic,system]);
  const set=(mic,system)=>page.evaluate(([microphone,system])=>{
    const f=window.closeFixture,id=f.recording.sessionId;
    f.queueState={lanes:{microphone:[],system:[]},closed:{},loss:{microphone:0,system:0}};
    f.inputStatus={schemaVersion:4,ownerId:id,revision:(f.inputStatus?.revision??0)+1,reportAgeMs:0,microphone,system,notices:[],noticeLoss:f.queueState.loss};
  },[mic,system]);
  const binding=(state="connected",sourceEpoch=1,interruptions=0,reason=null)=>({state,sourceEpoch,interruptions,reason});
  try{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();
    await page.getByRole("button",{name:"録音を開始",exact:true}).click();
    const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});
    await page.evaluate(()=>{window.closeFixture.refreshInputStatus=true;return window.closeFixture.recordChanged({phase:"recording",microphoneConnected:true,systemAudioConnected:true});});
    await states("unknown","unknown");results.push("legacy connected flags do not create a live input claim");
    await set(binding(),binding());await states("connected","connected");results.push("current queued schema binds both live tracks");
    await set(binding("disconnected",1,1,"device_disconnected"),binding("disconnected",1,1,"stream_stopped"));await states("disconnected","disconnected");
    check((await page.locator('[data-input-kind="microphone"]').innerText()).includes("選択した機器が未接続"),"microphone reason missing");
    check((await page.locator('[data-input-kind="system"]').innerText()).includes("取得処理が停止"),"system reason missing");
    results.push("each disconnected input retains its own cause without an invented save claim");
    await set(binding("connected",2,1),binding("connected",2,1));await states("connected","connected");results.push("both tracks recover only with new source epochs");
    await page.evaluate(()=>window.closeFixture.refreshInputStatus=false);await states("stale","stale");results.push("frozen paired report expires both live statuses");
    await page.goto("http://127.0.0.1:1420/?window=recorder");await page.waitForFunction(()=>!!window.closeFixture);await page.setViewportSize({width:360,height:116});
    await page.evaluate(()=>{window.closeFixture.refreshInputStatus=true;return window.closeFixture.recordChanged({phase:"recording",sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID()});});
    await set(binding("disconnected",1,1,"device_disconnected"),binding("disconnected",1,1,"stream_stopped"));await states("disconnected","disconnected");
    check(await page.locator(".mini-shell").evaluate(e=>e.scrollHeight<=e.clientHeight+1&&e.scrollWidth<=e.clientWidth+1),"mini input status overflow");
    check(await page.evaluate(()=>window.closeFixture.calls.every(c=>!["receive_recording_input_notice","render_recording_input_notice","close_recording_input_notice"].includes(c.command))),"mini consumed unseen notifications");
    results.push("mini reports both inputs without overflow or notification acknowledgement");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`);}finally{await page.context().unrouteAll();}
}
