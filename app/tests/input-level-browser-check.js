// Production recording workspace, synthetic IPC. No native app or audio capture.
async page=>{
  const results=[],check=(value,message)=>{if(!value)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();const body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const panel=()=>page.getByRole("region",{name:"録音状態",exact:true});
  const tracks=()=>panel().locator(".input-level");
  const waitStates=(a,b)=>page.waitForFunction(([a,b])=>{const t=document.querySelectorAll(".input-level");return t.length===2&&t[0].dataset.levelState===a&&t[1].dataset.levelState===b;},[a,b]);
  const report=async(a,b)=>page.evaluate(([a,b])=>{
    const f=window.closeFixture,unknown=s=>({state:s,windowFrames:null,ageMs:null,rms:null,peak:null});
    const track=value=>typeof value==="string"?unknown(value):{state:"measured",windowFrames:1024,ageMs:0,...value};
    f.inputLevels={schemaVersion:1,ownerId:f.recording.sessionId,revision:(f.inputLevels?.revision??0)+1,reportAgeMs:0,microphone:track(a),system:track(b)};
  },[a,b]);
  const start=async()=>{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();await page.locator("#recording-title").waitFor();
    await page.waitForFunction(()=>{const b=[...document.querySelectorAll('.recording-workspace button')].find(e=>e.textContent==="録音を開始");return b&&!b.disabled;});
    check(await page.evaluate(()=>window.closeFixture.calls.every(c=>c.command!=="get_recording_levels"&&c.command!=="start_recording")),"preparation started input telemetry or capture");
    await panel().getByRole("button",{name:"録音を開始",exact:true}).click();
    const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});
    await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});
    await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="get_recording_levels"));
  };
  try{
    await start();await waitStates("unavailable","unavailable");
    check(await panel().getByRole("meter").count()===0,"missing publication fabricated meters");
    results.push("opening preparation never reads levels or starts capture; only accepted canonical recording polls its session");
    const validOwner=await page.evaluate(()=>window.closeFixture.recording.sessionId);
    await page.evaluate(()=>window.closeFixture.recordChanged({sessionId:"invalid-synthetic-owner"}));
    await waitStates("unavailable","unavailable");
    const invalidOwnerReads=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_levels").length);
    await page.waitForTimeout(250);
    check(await panel().isVisible()&&await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_levels").length)===invalidOwnerReads,"invalid telemetry owner crashed recording UI or began reads");
    await page.evaluate(id=>window.closeFixture.recordChanged({sessionId:id}),validOwner);
    results.push("an invalid telemetry owner fails closed without crashing the recording workspace or requesting audio");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording",microphoneConnected:true,systemAudioConnected:true}));
    await report("waiting","waiting");await waitStates("waiting","waiting");
    check(await panel().getByRole("meter").count()===0,"connected tracks fabricated measurements");
    results.push("connection alone never creates a zero, moving meter or audio-success claim");
    await report({rms:0.25,peak:0.5},"waiting");await waitStates("measured","waiting");
    check(await panel().getByRole("meter",{name:"マイクのピーク入力レベル",exact:true}).getAttribute("value")==="0.5"&&(await tracks().nth(0).innerText()).includes("-12.0 dBFS"),"real amplitudes did not produce exact separate meters");
    results.push("microphone PCM drives its own peak and RMS dBFS while system remains unreceived");
    await report({rms:0,peak:0},{rms:0.5,peak:1});await waitStates("measured","measured");
    await page.waitForFunction(()=>document.querySelectorAll('.input-level')[0]?.textContent.includes("平均 無音"));
    check((await tracks().nth(0).innerText()).includes("ピーク 無音")&&(await tracks().nth(1).innerText()).includes("-6.0 dBFS"),"silence was unknown or fabricated finite dB");
    results.push("received silence is explicit silence with no fabricated finite dB, independently of audible system PCM");
    await report({rms:0.5,peak:2},"unavailable");await waitStates("measured","unavailable");
    check((await tracks().nth(0).innerText()).includes("6.0 dBFS")&&(await tracks().nth(0).innerText()).includes("音が大きすぎる")&&await panel().getByRole("meter").getAttribute("value")==="1","clipping evidence was hidden or meter exceeded full-scale");
    results.push("over-full-scale evidence remains in peak text with a warning; only the visual bar is clamped");
    await waitStates("stale","unavailable");
    check(await panel().getByRole("meter").count()===0,"re-reading immutable publication refreshed its age");
    results.push("unchanged publications age out after one second even while native polling continues");
    await report({rms:0.25,peak:0.5},"waiting");await waitStates("measured","waiting");
    await page.evaluate(()=>{window.closeFixture.inputLevels.microphone.rms=3;});await waitStates("unavailable","unavailable");
    check(await panel().getByRole("meter").count()===0,"invalid reply retained believable meters");
    results.push("impossible measurement shape invalidates both displays without reporting recording failure or success");
    await report({rms:0.25,peak:0.5},"waiting");await waitStates("measured","waiting");
    await page.evaluate(()=>{window.closeFixture.holdInputLevels=true;});
    await page.waitForFunction(()=>window.closeFixture.pendingInputLevels.length===1);
    const requests=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_levels").length);
    await waitStates("stale","waiting");
    check(await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_levels").length)===requests,"pending IPC allowed overlapping requests");
    results.push("a hung read never overlaps another poll and existing measurements still age out");
    await page.evaluate(async()=>{const f=window.closeFixture;await f.recordChanged({sessionId:crypto.randomUUID()});});
    await waitStates("unavailable","unavailable");
    await page.evaluate(()=>{const f=window.closeFixture;f.holdInputLevels=false;f.pendingInputLevels.splice(0).forEach(resolve=>resolve());});
    await report({rms:0.1,peak:0.2},"waiting");await waitStates("measured","waiting");
    check(await panel().getByRole("meter").getAttribute("value")==="0.2","late previous-owner reply overwrote current session");
    results.push("session change clears old values and late replies cannot overwrite the new session's PCM");
    await page.evaluate(()=>{const f=window.closeFixture;f.holdInputLevels=true;});
    await page.waitForFunction(()=>window.closeFixture.pendingInputLevels.length===1);
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"stopping"}));
    await waitStates("unavailable","unavailable");
    const stopped=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_levels").length);
    await page.evaluate(()=>{const f=window.closeFixture;f.holdInputLevels=false;f.pendingInputLevels.splice(0).forEach(resolve=>resolve());});
    await page.waitForTimeout(450);
    check(await panel().getByRole("meter").count()===0&&await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_levels").length)===stopped,"stopping recording continued polling or adopted a late reply");
    results.push("stopping immediately clears levels and stops reads; a late accepted reply cannot resurrect meters");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording"}));
    await report({rms:0.25,peak:0.5},{rms:0,peak:0});
    await page.evaluate(()=>{window.closeFixture.refreshInputLevels=true;});
    for(const width of [1280,960,360])for(const theme of ["light","dark"]){
      await page.setViewportSize({width,height:640});await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
      await page.waitForFunction(t=>document.documentElement.dataset.theme===t,theme);
      await waitStates("measured","measured");await panel().locator('.recording-input-table').scrollIntoViewIfNeeded();
      check(await panel().getByRole("meter").count()===2,"independent level meters disappeared");
      check(await panel().evaluate(e=>e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth),"level display horizontally overflows");
      await page.screenshot({path:`input-levels-${theme}-${width}.png`});
    }
    results.push("two independent meters and silence/writing disclaimer fit both themes at 1280/960/360px");
    // These explanations live in the production help popovers.
    await page.setViewportSize({width:1280,height:900});
    for(const [label,text] of [["録音の入力状態","保存の成功を示すものではありません"],["録音の書込み","入力レベルや接続状態だけでは保存中と表示しません"]]){
      await panel().getByRole("button",{name:`${label}の説明を開く`,exact:true}).click();
      const help=page.getByRole("dialog",{name:label,exact:true});await help.waitFor();
      check((await help.innerText()).includes(text),"telemetry claimed saved audio");
      await page.keyboard.press("Escape");await help.waitFor({state:"hidden"});
    }
    results.push("input evidence is explicitly separate from durable CAF writing and saving success");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator('body').innerText()}`);}
}
