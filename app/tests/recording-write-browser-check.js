// Product UI, synthetic IPC only. This does not start a native recorder.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');await route.fulfill({response,body});
  });
  const panel=()=>page.getByRole("region",{name:"録音の書込み",exact:true});
  const states=(a,b)=>page.waitForFunction(([a,b])=>{const t=document.querySelectorAll('[data-write-state]');return t.length===2&&t[0].dataset.writeState===a&&t[1].dataset.writeState===b;},[a,b]);
  const count=()=>page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_writes").length);
  const set=async(a,b)=>page.evaluate(([a,b])=>{const f=window.closeFixture,t=x=>typeof x==="string"?{state:x,frames:null,ageMs:null}:{state:"written",frames:x,ageMs:0};f.writes={schemaVersion:1,ownerId:f.recording.sessionId,revision:(f.writes?.revision??0)+1,microphone:t(a),system:t(b)};},[a,b]);
  try{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);await page.getByRole("button",{name:"新しい録音",exact:true}).click();
    await page.locator("#recording-title").waitFor();check(await count()===0,"preparation requested write evidence");
    await page.getByRole("button",{name:"録音を開始",exact:true}).click();const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});
    await panel().waitFor();await states("unavailable","unavailable");check(!(await panel().innerText()).includes("この端末に保存中"),"missing writes claimed storage");
    results.push("preparation never reads recording files; accepted session with unknown files never claims saving");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording",microphoneConnected:true,systemAudioConnected:true}));await set("waiting","waiting");await states("waiting","waiting");
    check(!(await panel().innerText()).includes("この端末に保存中"),"connection fabricated write progress");results.push("connected inputs without file PCM remain separate waiting writes");
    await set(480,"waiting");await states("written","waiting");check(!(await panel().innerText()).includes("この端末に保存中"),"one input claimed both saved");results.push("one written track is explicit and cannot claim both inputs are saving");
    await set(960,960);await states("written","written");check((await panel().innerText()).includes("この端末に保存中"),"two fresh writes did not reach saving status");
    await states("stale","stale");check(!(await panel().innerText()).includes("この端末に保存中"),"unchanged polling freshened write");results.push("both fresh file writes show saving and unchanged evidence expires after one second");
    await page.evaluate(()=>{const f=window.closeFixture;f.writes.revision++;});await page.waitForTimeout(650);await states("stale","stale");results.push("a new RPC revision or touched metadata without additional frames cannot freshen storage");
    await set(1440,1440);await states("written","written");await page.evaluate(()=>window.closeFixture.failWrites=true);await states("unavailable","unavailable");
    await page.evaluate(()=>window.closeFixture.failWrites=false);await set(1920,1920);await states("written","written");results.push("failed reads clear saved claims and later verified growth can recover");
    await page.evaluate(()=>window.closeFixture.holdWrites=true);await page.waitForFunction(()=>window.closeFixture.pendingWrites.length===1);const held=await count();await states("stale","stale");check(await count()===held,"hung write read overlapped another poll");
    await page.evaluate(()=>window.closeFixture.recordChanged({sessionId:crypto.randomUUID()}));await states("unavailable","unavailable");
    await page.evaluate(()=>{const f=window.closeFixture;f.holdWrites=false;f.pendingWrites.splice(0).forEach(resolve=>resolve());});await set(480,480);await states("written","written");results.push("a hung read does not overlap and a late prior-session result cannot replace the current owner");
    const valid=await page.evaluate(()=>window.closeFixture.recording.sessionId);await page.evaluate(()=>window.closeFixture.recordChanged({sessionId:"invalid-owner"}));await states("unavailable","unavailable");const invalid=await count();await page.waitForTimeout(650);check(await count()===invalid&&await panel().isVisible(),"invalid owner crashed UI or read audio");
    await page.evaluate(id=>window.closeFixture.recordChanged({sessionId:id}),valid);await set(960,960);await states("written","written");results.push("invalid owners fail closed without filesystem reads or crashing the recorder UI");
    await page.evaluate(()=>window.closeFixture.holdWrites=true);await page.waitForFunction(()=>window.closeFixture.pendingWrites.length===1);await page.evaluate(()=>window.closeFixture.recordChanged({phase:"stopping"}));await states("unavailable","unavailable");const stopped=await count();
    await page.evaluate(()=>{const f=window.closeFixture;f.holdWrites=false;f.pendingWrites.splice(0).forEach(resolve=>resolve());});await page.waitForTimeout(650);check(await count()===stopped,"stop retained write polling");results.push("stopping clears the write claim and a late read cannot resurrect saving");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording"}));await set(1440,1440);await page.evaluate(()=>window.closeFixture.refreshWrites=true);
    for(const [width,theme] of [[1280,"light"],[960,"dark"],[360,"dark"]]){
      await page.setViewportSize({width,height:900});await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);await states("written","written");await panel().scrollIntoViewIfNeeded();
      check(await panel().evaluate(e=>e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth+1),`write display overflow ${width}`);
      await page.screenshot({path:`recording-writes-${width}-${theme}.png`,fullPage:true});results.push(`write evidence fits ${width}px ${theme}`);
    }
    check((await panel().innerText()).includes("停止後に確認")&&(await panel().innerText()).includes("電源断"),"write evidence claimed finalization or power-loss durability");results.push("current file writing is distinct from recording finalization and power-loss durability");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`);}finally{await page.context().unrouteAll();}
}
