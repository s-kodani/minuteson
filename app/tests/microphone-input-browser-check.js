async (page, capturePath, detailUI) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);},context=page.context();
  await context.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();
    const body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const input=()=>page.getByRole("combobox",{name:"録音するマイク",exact:true});
  const start=()=>page.getByRole("button",{name:"録音を開始",exact:true});
  const refresh=()=>page.getByRole("button",{name:"マイク一覧を再確認",exact:true});
  const section=()=>page.getByRole("region",{name:"録音の入力準備",exact:true});
  const title="選択した会議の録音を開始しますか？",modal=()=>page.getByRole("dialog",{name:title,exact:true});
  const accept=()=>modal().getByRole("button",{name:"録音を開始する",exact:true});
  const count=command=>page.evaluate(c=>window.closeFixture.calls.filter(x=>x.command===c).length,command);
  const boot=async patch=>{
    await page.goto("http://127.0.0.1:1420/");
    await page.waitForFunction(()=>!!window.closeFixture&&!!document.querySelector(".meeting-table tbody tr"));
    if(patch)await page.evaluate(p=>Object.assign(window.closeFixture.captureInputs,p),patch);
    await page.locator('.sidebar').getByRole("button",{name:"新しい録音",exact:true}).click();
    await input().waitFor();
    await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するマイク"]')?.disabled);
  };
  const dismissFailure=async()=>{await modal().getByRole("alert").waitFor();await modal().getByRole("button",{name:"確認を閉じる",exact:true}).click();await modal().waitFor({state:"hidden"});};
  let stage="setup";
  try{
    stage="metadata-only";await boot();
    check(await input().inputValue()===""&&await count("get_capture_inputs")===1&&await count("start_recording")===0,"initial catalog started capture or failed automatic choice");
    check((await section().innerText()).includes("実際の音声入力はまだ確認していません")&&(await section().innerText()).includes("5秒"),"metadata fabricated tested audio");
    results.push("metadata-only catalog starts in automatic mode without pinning the OS default or starting recording");

    stage="explicit-cancel";await input().selectOption("usb-123");await start().click();await modal().waitFor();
    check((await modal().innerText()).includes("ヘッドセットのマイク")&&(await modal().innerText()).includes("PC音声は全体取得"),"confirmation omitted actual microphone or scope");
    check(await input().isDisabled()&&await refresh().isDisabled(),"selection remained mutable during consent");
    await page.keyboard.press("Escape");await modal().waitFor({state:"hidden"});
    check(await count("start_recording")===0&&await input().inputValue()==="usb-123","declining started capture or erased choice");
    results.push("explicit microphone and global PC scope are reviewed; cancellation keeps the selected UID without capture");

    stage="exact-start";await start().click();await accept().dblclick();await modal().waitFor({state:"hidden"});
    check(await count("start_recording")===1&&await page.evaluate(()=>window.closeFixture.calls.find(c=>c.command==="start_recording").args.microphoneUid==="usb-123"),"start lost stable UID or repeated request");
    results.push("consented double activation sends exactly one start with the exact microphone UID");

    stage="active-switch";
    const active=page.getByRole("combobox",{name:"録音中のマイク",exact:true});
    await active.waitFor();await active.selectOption("built-in");
    await page.waitForFunction(()=>window.closeFixture.microphoneSelection?.mode==="fixed"&&window.closeFixture.microphoneSelection.deviceUID==="built-in");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording"}));
    await active.selectOption("");
    await page.waitForFunction(()=>window.closeFixture.microphoneSelection?.mode==="automatic"&&window.closeFixture.microphoneSelection.requestRevision===2);
    check(await count("start_recording")===1&&await count("select_recording_microphone")===2,"microphone switching restarted capture or duplicated a request");
    results.push("preparing and recording allow fixed/automatic microphone switching without restarting capture");

    stage="saved-default-changed";await boot({selectedMicrophoneUID:"usb-123"});
    check(await input().inputValue()===""&&await start().isEnabled(),"old selected UID pinned the automatic default");
    results.push("a previously selected UID does not override the new automatic initial mode");

    stage="fixed-missing";await boot();await input().selectOption("usb-123");
    await page.evaluate(()=>{window.closeFixture.captureInputs.devices=window.closeFixture.captureInputs.devices.filter(d=>d.uid!=="usb-123");});await refresh().click();
    await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するマイク"]')?.disabled);
    check(await input().inputValue()==="usb-123"&&await start().isDisabled()&&(await section().innerText()).includes("見つかりません"),"missing fixed device silently fell back");
    await input().selectOption("built-in");check(await start().isEnabled(),"explicit alternate device did not enable consent");
    results.push("disconnected saved UID blocks start until an alternate is explicitly selected");

    stage="same-name";await boot({devices:[{uid:"one",name:"同じマイク名",isDefault:true},{uid:"two",name:"同じマイク名",isDefault:false}]});
    await input().selectOption("two");await start().click();await accept().click();await modal().waitFor({state:"hidden"});
    check(await page.evaluate(()=>window.closeFixture.calls.find(c=>c.command==="start_recording").args.microphoneUid==="two"),"same-named devices confused identity");
    results.push("same-named microphones remain distinct by UID rather than label or list position");

    for(const permission of ["denied","restricted"]){
      stage=permission;await boot({microphonePermission:permission});
      check(await start().isDisabled()&&(await section().innerText()).includes("macOSのシステム設定")&&await count("start_recording")===0,"denied permission allowed capture or requested TCC");
      results.push(`${permission} microphone permission blocks start and provides manual permission guidance`);
    }
    stage="unknown";await boot({microphonePermission:"unknown"});
    check(await start().isEnabled()&&(await section().innerText()).includes("状態不明")&&!(await section().innerText()).includes("権限は許可済み"),"unknown permission fabricated authorization");
    results.push("unknown permission stays explicitly unknown and does not claim verified input");

    stage="refresh-failure";await boot();await input().selectOption("usb-123");
    await page.evaluate(()=>{window.closeFixture.failCaptureInputs=true;});await refresh().click();
    await section().getByRole("alert").waitFor();check(await start().isDisabled()&&await input().inputValue()==="usb-123","read failure enabled capture or changed choice");
    await page.evaluate(()=>{window.closeFixture.failCaptureInputs=false;});await refresh().click();
    await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するマイク"]')?.disabled);
    check(await start().isEnabled()&&await input().inputValue()==="usb-123","explicit refresh failed to preserve choice");
    results.push("catalog failure blocks capture and explicit successful refresh preserves the UID");

    stage="malformed";await boot();await page.evaluate(()=>{window.closeFixture.captureInputs.devices.push({...window.closeFixture.captureInputs.devices[0]});});await refresh().click();
    await section().getByRole("alert").waitFor();check(await start().isDisabled()&&await count("start_recording")===0,"duplicate device metadata accepted");
    results.push("duplicate or malformed catalog cannot enable recording");

    for(const change of ["disconnected","permission"]){
      stage=`confirm-${change}`;await boot();await input().selectOption("usb-123");
      await page.evaluate(async()=>{await window.closeFixture.importChanged({token:crypto.randomUUID(),phase:"ready",busy:false,fileName:"保持する音声.m4a"});});
      await start().click();await accept().waitFor();
      await page.evaluate(change=>{if(change==="disconnected")window.closeFixture.captureInputs.devices=window.closeFixture.captureInputs.devices.filter(d=>d.uid!=="usb-123");else window.closeFixture.captureInputs.microphonePermission="denied";},change);
      await accept().click();await dismissFailure();
      check(await count("start_recording")===0&&await count("cancel_audio_import")===0&&await page.evaluate(()=>window.closeFixture.importing.fileName==="保持する音声.m4a"),"post-consent device change started or cancelled import");
      results.push(`post-confirmation ${change} prevents start and preserves selected import without cancellation`);
    }

    stage="late-after-navigation";await boot();await input().selectOption("built-in");
    await page.evaluate(()=>{window.closeFixture.holdCaptureInputs=true;window.closeFixture.captureInputs.devices=[];});await refresh().click();
    await page.waitForFunction(()=>window.closeFixture.pendingInputs.length===1);
    await page.locator('.sidebar').getByRole("button",{name:"会議一覧",exact:true}).click();
    await page.evaluate(()=>{window.closeFixture.holdCaptureInputs=false;window.closeFixture.captureInputs.devices=[{uid:"new",name:"新しいマイク",isDefault:true}];window.closeFixture.pendingInputs.splice(0).forEach(resolve=>resolve());});
    await page.locator('.sidebar').getByRole("button",{name:"新しい録音",exact:true}).click();
    await page.waitForFunction(()=>!document.querySelector('[aria-label="録音するマイク"]')?.disabled);
    check(await input().inputValue()==="built-in"&&await start().isDisabled()&&await input().locator('option[value="new"]').count()===1,"late response replaced new catalog or silently selected replacement");
    results.push("late catalog response after leaving a meeting is discarded; new catalog retains missing previous choice");

    stage="layout";await boot({selectedMicrophoneUID:"usb-123"});
    for(const [theme,width] of [["light",1280],["dark",960]]){
      await page.setViewportSize({width,height:900});await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;},theme);
      await section().scrollIntoViewIfNeeded();
      check(await section().evaluate(e=>e.scrollWidth<=e.clientWidth+1),"input controls horizontally overflow");
      await page.screenshot({path:`microphone-${theme}-${width}.png`});
      await section().getByRole("heading",{name:"マイク",exact:true}).scrollIntoViewIfNeeded();
      await page.screenshot({path:`microphone-${theme}-${width}-top.png`});
      await section().locator("p").last().scrollIntoViewIfNeeded();
      await page.screenshot({path:`microphone-${theme}-${width}-bottom.png`});
    }
    results.push("input preparation remains readable without horizontal clipping in light 1280px and dark 960px layouts");
    return {passed:results.length,results};
  }catch(cause){throw Error(`${stage}: ${cause}; body=${await page.locator("body").innerText()}`);}
  finally{await page.goto("about:blank");await context.unrouteAll();}
}
