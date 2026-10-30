async (page, capturePath, detailUI) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);},context=page.context();
  await context.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();
    const body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const mode=()=>page.getByRole("combobox",{name:"PC音声の取得範囲",exact:true,includeHidden:true});
  const app=()=>page.getByRole("combobox",{name:"録音するアプリ",exact:true,includeHidden:true});
  const start=()=>page.getByRole("button",{name:"録音を開始",exact:true});
  const refresh=()=>page.getByRole("button",{name:"アプリ一覧を再確認",exact:true,includeHidden:true});
  const details=()=>page.getByRole("region",{name:"PC音声の取得設定",exact:true});
  const begin=async()=>{await start().click();};
  const section=()=>page.getByRole("region",{name:"録音の入力準備",exact:true});
  const modal=()=>page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});
  const accept=()=>modal().getByRole("button",{name:"録音を開始する",exact:true});
  const count=command=>page.evaluate(c=>window.closeFixture.calls.filter(x=>x.command===c).length,command);
  const boot=async()=>{
    await page.goto("http://127.0.0.1:1420/");
    await page.waitForFunction(()=>!!window.closeFixture&&!!document.querySelector(".meeting-table tbody tr"));
    await page.locator('.sidebar').getByRole("button",{name:"新しい録音",exact:true}).click();
    await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するマイク"]')?.disabled);

  };
  const choose=async(index=0)=>{

    await mode().selectOption("application");
    await page.waitForFunction(()=>{const e=document.querySelector('[aria-label="録音するアプリ"]');return e&&!e.disabled;});
    const key=await page.evaluate(i=>{const a=window.closeFixture.captureApplications.applications[i];return JSON.stringify([a.pid,a.bundleIdentifier,a.launchTimeUs]);},index);
    await app().selectOption(key);
    return key;
  };
  const dismissFailure=async()=>{await page.getByRole("alert").filter({hasText:"処理を完了できませんでした"}).first().waitFor();await modal().getByRole("button",{name:"確認を閉じる",exact:true}).click();await modal().waitFor({state:"hidden"});};
  let stage="setup";
  try{
    stage="global";await boot();await begin();await accept().click();await modal().waitFor({state:"hidden"});
    check(await count("get_capture_applications")===0&&await page.evaluate(()=>{const s=window.closeFixture.calls.find(c=>c.command==="start_recording").args.captureScope;return s.mode==="system"&&s.application===null;}),"global mode enumerated applications or lost explicit scope");
    results.push("global recording submits explicit system scope without enumerating applications");

    stage="explicit-choice";await boot();await mode().selectOption("application");
    await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するアプリ"]')?.disabled);
    check(await start().isDisabled()&&await app().inputValue()===""&&await count("start_recording")===0,"application automatically chosen or capture started");
    check((await details().innerText()).includes("権限未確認")&&!(await details().innerText()).includes("PC音声の権限は許可済み"),"catalog fabricated authorization");
    results.push("application scope stays unset until explicit choice; metadata does not claim input or permission verification");

    stage="cancel";const key=await choose(1);await begin();await modal().waitFor();
    check(await modal().getByRole("combobox",{name:"会議全体の話者数",exact:true}).isVisible()&&await mode().isDisabled()&&await app().isDisabled()&&await refresh().isDisabled(),"confirmation omitted app scope or left it mutable");
    await page.keyboard.press("Escape");await modal().waitFor({state:"hidden"});
    check(await app().inputValue()===key&&await mode().inputValue()==="application"&&await count("start_recording")===0,"cancel erased target or started");
    results.push("speaker confirmation locks the application scope; cancellation keeps exact choice without capture");

    stage="same-name-exact-start";await begin();await accept().dblclick();await modal().waitFor({state:"hidden"});
    check(await count("start_recording")===1&&await page.evaluate(()=>{const a=window.closeFixture.calls.find(c=>c.command==="start_recording").args.captureScope;return a.mode==="application"&&a.application.pid===456&&a.application.bundleIdentifier==="com.example.other"&&a.application.launchTimeUs===1791100000000002;}),"same-named target confused or double start sent");
    results.push("double consent sends one request with exact PID, bundle and process start time for same-named applications");

    for(const change of ["restart","exit","bundle"]){
      stage=`confirm-${change}`;await boot();await choose();
      await page.evaluate(async()=>{await window.closeFixture.importChanged({token:crypto.randomUUID(),phase:"ready",busy:false,fileName:"保持する音声.m4a"});});
      await begin();await accept().waitFor();
      await page.evaluate(change=>{const f=window.closeFixture;if(change==="exit")f.captureApplications.applications.shift();else if(change==="restart")f.captureApplications.applications[0].launchTimeUs++;else f.captureApplications.applications[0].bundleIdentifier="com.example.replaced";},change);
      await accept().click();await dismissFailure();
      check(await count("start_recording")===0&&await count("cancel_audio_import")===0&&await mode().inputValue()==="application"&&await page.evaluate(()=>window.closeFixture.importing.fileName==="保持する音声.m4a"),"post-consent changed target captured, fell back or cancelled import");
      results.push(`post-consent ${change} rejects target before start and preserves existing import and application mode`);
    }

    stage="flush-restart";await boot();await choose();await begin();await accept().waitFor();
    await page.evaluate(()=>{window.closeFixture.mutateApplicationDuringFlush=true;});await accept().click();await dismissFailure();
    check(await count("start_recording")===0&&await count("cancel_audio_import")===0,"post-edit-flush restart was not rechecked");
    results.push("process restart while saving edits is rejected by fresh post-flush catalog");

    stage="refresh-error";await boot();const selected=await choose();await page.evaluate(()=>{window.closeFixture.failCaptureApplications=true;});await refresh().click();
    await page.getByRole("alert").filter({hasText:"アプリ一覧を確認できません"}).waitFor();
    check(await start().isDisabled()&&await mode().inputValue()==="application"&&await app().inputValue()===selected,"catalog error changed target or enabled start");
    await page.evaluate(()=>{window.closeFixture.failCaptureApplications=false;});await refresh().click();
    await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するアプリ"]')?.disabled);
    check(await start().isEnabled()&&await app().inputValue()===selected,"explicit successful refresh lost target");
    results.push("catalog failure blocks capture without fallback; successful explicit refresh restores the same exact instance");

    stage="malformed";await boot();await choose();await page.evaluate(()=>{window.closeFixture.captureApplications.applications.push({...window.closeFixture.captureApplications.applications[0]});});await refresh().click();
    await page.getByRole("alert").filter({hasText:"アプリ一覧を確認できません"}).waitFor();
    check(await start().isDisabled()&&await count("start_recording")===0,"ambiguous catalog enabled capture");
    results.push("duplicate PID metadata blocks application capture instead of selecting by name");

    stage="missing-before-consent";await boot();await choose();await page.evaluate(()=>{window.closeFixture.captureApplications.applications=[];});await begin();
    await page.getByRole("alert").filter({hasText:"録音の操作"}).waitFor();
    check((await page.getByRole("alert").filter({hasText:"録音の操作"}).innerText()).includes("確認したアプリが終了または再起動しました"),"pre-consent error did not identify changed application");
    check(await modal().count()===0&&await count("start_recording")===0&&await mode().inputValue()==="application","fresh check before consent fell back or started");
    results.push("fresh catalog rejects missing target before consent without global fallback");

    stage="late-navigation";await boot();await choose();await page.evaluate(()=>{window.closeFixture.holdCaptureApplications=true;});await refresh().click();
    await page.waitForFunction(()=>window.closeFixture.pendingApplications.length===1);await page.locator('.sidebar').getByRole("button",{name:"会議一覧",exact:true}).click();
    await page.evaluate(()=>{const f=window.closeFixture;f.holdCaptureApplications=false;f.captureApplications.applications=[];f.pendingApplications.splice(0).forEach(resolve=>resolve());});
    await page.locator('.sidebar').getByRole("button",{name:"新しい録音",exact:true}).click();

    await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するアプリ"]')?.disabled);
    check(await app().locator("option").count()===1&&await start().isDisabled()&&await mode().inputValue()==="application","late old catalog replaced new meeting catalog");
    results.push("late catalog after navigation is discarded; missing target remains explicit and cannot start");

    stage="lost-ack";await boot();await choose();await page.evaluate(()=>{window.closeFixture.startLost=true;});await begin();await accept().click();await dismissFailure();
    check(await count("start_recording")===1&&await start().count()===0&&await page.evaluate(()=>window.closeFixture.recording.phase==="preparing"),"lost acknowledgement replayed start or ignored canonical state");
    results.push("lost start acknowledgement retains canonical preparing state and never replays the request");

    stage="native-last-hop";await boot();await choose();await page.evaluate(()=>{window.closeFixture.mutateApplicationAtStart=true;});await begin();await accept().click();await dismissFailure();
    check(await count("start_recording")===1&&await page.evaluate(()=>window.closeFixture.recording.phase==="idle"),"synthetic native guard accepted restarted target");
    results.push("synthetic last-hop native rejection never publishes preparing or changes to global scope");

    stage="layout";await boot();await choose();
    for(const [theme,width] of [["light",1280],["dark",960]]){
      await page.setViewportSize({width,height:900});await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;},theme);
      await app().scrollIntoViewIfNeeded();
      check(await details().evaluate(e=>e.scrollWidth<=e.clientWidth+1),"application controls horizontally overflow");
      await page.screenshot({path:`application-scope-${theme}-${width}.png`});
      await details().locator("p:visible").last().scrollIntoViewIfNeeded();
      await page.screenshot({path:`application-scope-${theme}-${width}-bottom.png`});
    }
    results.push("production application scope is readable in light 1280px and dark 960px layouts without horizontal overflow");
    return {passed:results.length,results};
  }catch(cause){throw Error(`${stage}: ${cause}; body=${await page.locator("body").innerText()}`);}
  finally{await page.goto("about:blank");await context.unrouteAll();}
}
