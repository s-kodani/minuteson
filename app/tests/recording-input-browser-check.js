// Both product windows, synthetic IPC. No native capture or audio output.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');await route.fulfill({response,body});
  });
  const status=()=>page.locator("[data-input-state]"),state=async value=>page.waitForFunction(v=>document.querySelector("[data-input-state]")?.dataset.inputState===v,value);
  const count=()=>page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_input_status").length);
  const set=async(value,epoch,count,target=null,reason=null)=>page.evaluate(([value,epoch,count,target,reason])=>{
    const f=window.closeFixture,id=f.recording.sessionId,kind=value==="connected"?"recovered":"interrupted";
    f.inputStatus={schemaVersion:1,ownerId:id,revision:(f.inputStatus?.revision??0)+1,reportAgeMs:0,microphone:{state:value,sourceEpoch:epoch,interruptions:count,reason},notice:target?{id:`${id}:microphone:${count}:${kind}`,target,kind,interruptions:count}:null};
  },[value,epoch,count,target,reason]);
  const toast=()=>page.locator(".toast-card").filter({hasText:/マイクの取得|同じマイク/u});
  try{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();await page.locator("#recording-title").waitFor();check(await count()===0,"preparation requested active binding state");
    await page.getByRole("button",{name:"録音を開始",exact:true}).click();const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording",microphoneConnected:true,systemAudioConnected:true}));await state("unknown");
    check(!(await status().innerText()).includes("接続済み"),"legacy flag fabricated connection");results.push("only accepted current recordings are polled; legacy flags cannot claim current binding");
    await set("connected",1,0);await state("connected");check(await toast().count()===0,"initial connection claimed recovery");results.push("initial connection has no false recovery notification");
    await page.evaluate(()=>window.closeFixture.refreshInputStatus=true);await set("disconnected",1,1,"main","device_disconnected");await state("disconnected");await toast().waitFor();
    const notification=await toast().getAttribute("data-toast-id");await page.waitForTimeout(1100);check(await toast().getAttribute("data-toast-id")===notification,"heartbeat recreated notice");
    await toast().getByRole("button",{name:"注意通知を閉じる",exact:true}).click();await page.waitForTimeout(650);check(await toast().count()===0&&await status().getAttribute("data-input-state")==="disconnected","dismissal removed state or polling resurrected notice");results.push("heartbeats do not duplicate notices; dismissal retains the issue");
    await set("unavailable",1,1,"main","format_changed");await state("unavailable");check((await status().innerText()).includes("入力形式が変更"),"format condition hidden");check(await toast().count()===0,"reason change repeated warning");results.push("format changes stay explicit without repeating the same interruption");
    await page.evaluate(()=>window.closeFixture.failInputStatus=true);await state("unknown");await page.evaluate(()=>window.closeFixture.failInputStatus=false);await state("unavailable");check(await toast().count()===0,"read failure erased dismissal history");results.push("temporary read failure does not redeliver the same dismissed interruption");
    for(const [width,theme] of [[1280,"light"],[960,"dark"],[360,"dark"]]){
      await page.setViewportSize({width,height:900});await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);await state("unavailable");await status().scrollIntoViewIfNeeded();
      check(await status().evaluate(e=>e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth+1),`retained binding status overflow ${width}`);await page.screenshot({path:`input-status-main-${width}-${theme}.png`});results.push(`retained issue fits ${width}px ${theme}`);
    }
    await page.getByRole("button",{name:"設定",exact:true}).click();await page.getByRole("button",{name:"録音画面へ",exact:true}).click();await state("unavailable");check(await toast().count()===0,"workspace remount resurrected notice");results.push("notification ownership survives switching settings and returning");
    await set("reconnecting",2,1,"main","restart_failed");await state("reconnecting");await set("connected",2,1,"recorder");await state("connected");check(await toast().count()===0,"main displayed recorder-routed recovery");results.push("new binding is retained but only its assigned window receives recovery notice");
    await page.evaluate(()=>window.closeFixture.failInputStatus=true);await state("unknown");await page.evaluate(()=>window.closeFixture.failInputStatus=false);await state("connected");results.push("failed reads clear connection and fresh authoritative reads restore it");
    await page.evaluate(()=>window.closeFixture.holdInputStatus=true);await page.waitForFunction(()=>window.closeFixture.pendingInputStatus.length===1);const held=await count();await state("stale");check(await count()===held,"hung RPC overlapped another poll");
    await page.evaluate(()=>window.closeFixture.recordChanged({sessionId:crypto.randomUUID()}));await state("unknown");await page.evaluate(()=>{const f=window.closeFixture;f.holdInputStatus=false;f.pendingInputStatus.splice(0).forEach(resolve=>resolve());});await set("connected",1,0);await state("connected");results.push("hung reads are bounded and late replies cannot cross owners");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"stopping"}));await state("unknown");const stopped=await count();await page.waitForTimeout(650);check(await count()===stopped,"stopped session kept polling");results.push("stopping clears connection and ceases telemetry reads");
    await page.goto("http://127.0.0.1:1420/?window=recorder");await page.waitForFunction(()=>!!window.closeFixture);await page.setViewportSize({width:360,height:116});
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording",sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID()}));await page.evaluate(()=>window.closeFixture.refreshInputStatus=true);
    await set("disconnected",1,1,"main","device_disconnected");await state("disconnected");check(await page.locator(".mini-input-notice").count()===0,"mini duplicated main notice");results.push("mini retains interruption without duplicating a main-routed notice");
    await set("disconnected",2,2,"recorder","device_disconnected");await page.locator(".mini-input-notice").waitFor();
    check(await page.locator(".mini-shell").evaluate(e=>e.scrollHeight<=e.clientHeight+1&&e.scrollWidth<=e.clientWidth+1),"mini overflowed 360 by 116 geometry");await page.screenshot({path:"input-status-mini-interrupted-360x116.png"});
    await page.getByRole("button",{name:"マイク通知を閉じる",exact:true}).click();await page.waitForTimeout(650);check(await page.locator(".mini-input-notice").count()===0&&await status().getAttribute("data-input-state")==="disconnected","mini dismissal erased state or repeated notice");results.push("mini warning fits native geometry and dismissal retains state without repetition");
    await set("reconnecting",3,2,"recorder","restart_failed");await state("reconnecting");await set("connected",3,2,"recorder");await state("connected");await page.locator(".mini-input-notice").waitFor();
    check((await page.locator(".mini-input-notice").innerText()).includes("復帰"),"mini missed recovery");check(await page.locator(".mini-shell").evaluate(e=>e.scrollHeight<=e.clientHeight+1),"mini recovery overflowed");await page.screenshot({path:"input-status-mini-recovered-360x116.png"});results.push("mini shows verified recovery without claiming saving or continuous PCM");
    await page.evaluate(()=>window.closeFixture.failInputStatus=true);await state("unknown");await page.evaluate(()=>window.closeFixture.failInputStatus=false);await state("connected");check(await page.locator(".mini-input-notice").count()===0,"readback repeated mini recovery");results.push("mini recovery is delivered once even across an intervening unavailable interval");
    await page.evaluate(()=>window.closeFixture.refreshInputStatus=false);await state("stale");check(await page.locator(".mini-input-notice").count()===0,"stale report retained current recovery notice");results.push("stalled publications expire and cannot retain current recovery claims");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`);}finally{await page.context().unrouteAll();}
}
