async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const boot=async(platform="windows")=>{await page.goto(`http://127.0.0.1:1420/?resident=${platform}`);await page.waitForFunction(()=>!!window.closeFixture&&!!document.querySelector(".meeting-table tbody tr"));};
  const count=command=>page.evaluate(command=>window.closeFixture.calls.filter(c=>c.command===command).length,command);
  const request=quit=>page.evaluate(quit=>window.closeFixture.request(quit),quit);
  const background=()=>page.getByRole("dialog",{name:"バックグラウンドで継続しますか？",exact:true});
  const accept=()=>background().getByRole("button",{name:"継続して画面を閉じる",exact:true});
  const saving=phase=>page.evaluate(phase=>window.closeFixture.recordChanged({phase,sessionId:crypto.randomUUID(),meetingId:window.dataFixture.meetings[0].id}),phase);
  try{
    await boot();
    const monitor=()=>page.locator('.toast-card').filter({hasText:"常駐監視:"});
    const operation=()=>page.locator('.toast-card').filter({hasText:"常駐の操作:"});
    await page.evaluate(()=>window.emitResidencyFixture("residency-error","合成トレイ障害"));await operation().waitFor();
    await page.evaluate(()=>window.emitResidencyFixture("residency-monitor-status","合成パイプ障害"));await page.clock.runFor(300);
    check(await monitor().count()===0,"internal residency monitor notified");
    await page.evaluate(()=>window.emitResidencyFixture("residency-monitor-status",null));
    check(await operation().count()===1,"monitor recovery erased unrelated operation failure");
    results.push("automatic residency monitor stays silent and preserves failed user operations");
    for(const platform of ["windows","macos"]){
      await boot(platform);await request(false);await page.waitForFunction(()=>window.closeFixture.hidden);
      check(await count("hide_main_to_background")===1&&await count("finish_main_close")===0,"quiet X exited instead of hiding");
      results.push(`${platform}: idle close stores edits and hides through the guarded resident command`);
    }
    for(const phase of ["recording","saving"]){
      await boot();await saving(phase);await request(false);await background().waitFor();await page.keyboard.press("Enter");await background().waitFor({state:"hidden"});
      check(await count("hide_main_to_background")===0&&await count("stop_recording")===0&&await count("finish_main_close")===0,"declining stopped or hid work");
      await request(false);await accept().waitFor();await accept().dblclick();await page.waitForFunction(()=>window.closeFixture.hidden);
      check(await count("hide_main_to_background")===1&&await count("stop_recording")===0&&await count("finish_main_close")===0,"background consent stopped or duplicated work");
      results.push(`${phase}: X confirms continuation, cancellation retains UI, acceptance never resends stop`);
    }
    await boot();await page.evaluate(()=>{window.closeFixture.activity.analysisBusy=true;});await request(false);await accept().click();await page.waitForFunction(()=>window.closeFixture.hidden);
    check(await count("cancel_audio_import")===0&&await count("cancel_data_operation")===0&&await count("finish_main_close")===0,"hiding analysis cancelled jobs");
    results.push("analysis continues in background without cancellation or exit commands");

    for (const platform of ["windows", "macos"]) {
      await boot(platform);await page.evaluate(()=>{window.closeFixture.activity.recordingRecoveryBusy=true;});
      await request(false);await background().getByText(/録音の回復/).waitFor();
      await accept().click();await page.waitForFunction(()=>window.closeFixture.hidden);
      check(await count("hide_main_to_background")===1&&await count("finish_main_close")===0,"recovery background storage exited app");
      results.push(`${platform}: recovery alone requires background continuation consent`);
    }

    await boot();await page.evaluate(()=>{window.closeFixture.residency={supported:true,canBackground:false,trayAvailable:false,dockAvailable:false,error:"画面を開き直せません"};});await request(false);
    await page.getByRole("alert").filter({hasText:"画面を開き直せません"}).waitFor();check(await count("hide_main_to_background")===0&&await count("finish_main_close")===0,"missing tray hid window");
    results.push("no return path preserves the window before native hide");

    await boot();await saving("saving");await request(false);await accept().waitFor();await page.evaluate(()=>{window.closeFixture.hideFailure=true;});await accept().click();await page.getByRole("alert").filter({hasText:"合成の常駐入口消失"}).waitFor();
    check(await count("hide_main_to_background")===1&&await count("finish_main_close")===0&&!await page.evaluate(()=>!!window.closeFixture.hidden),"last-hop tray loss exited or falsely hid app");
    results.push("tray loss between review and hide retains UI and exposes failure");

    await boot();await saving("saving");await request(true);const dialog=page.getByRole("dialog",{name:"録音の保存完了後の終了を確認",exact:true});await dialog.getByRole("button",{name:"保存完了を待って終了へ進む",exact:true}).click();await dialog.getByRole("status").waitFor();
    check(await count("hide_main_to_background")===0&&await count("finish_main_close")===0,"tray quit bypassed save wait");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"idle",sessionId:null,meetingId:null}));await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="finish_main_close"));
    check(await count("finish_main_close")===1,"tray quit failed checked exit");results.push("explicit quit keeps the actual save-success barrier");

    await boot();await saving("saving");await request(false);await accept().waitFor();await page.evaluate(()=>{window.closeFixture.hideLost=true;});await accept().click();await page.getByRole("alert").filter({hasText:"合成の収納応答喪失"}).waitFor();
    check(await count("hide_main_to_background")===1&&await accept().isDisabled(),"lost hide response was replayable");
    results.push("unknown hide acknowledgement is visible and never automatically replayed");
    return{passed:results.length,results};
  }finally{await page.goto("about:blank");await page.context().unrouteAll();}
}
