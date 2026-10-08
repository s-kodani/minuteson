// Production recovery/close UI with synthetic IPC only. No native process or recording.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const clear={schemaVersion:1,revision:2,phase:"clear",runId:null,errorMessage:null};
  const attention={...clear,phase:"attention",runId:"735027d4-a03e-43d3-830a-21e767dfaf88",errorMessage:"模擬取得側の終了未確認・音声を保持"};
  const panel=()=>page.getByRole("region",{name:"テスト音声の回復",exact:true});
  const retry=()=>panel().getByRole("button",{name:"終了と所有を再検査して回収",exact:true});
  const count=command=>page.evaluate(command=>window.closeFixture.calls.filter(c=>c.command===command).length,command);
  const boot=async seed=>{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    if(seed){await page.evaluate(seed=>sessionStorage.setItem("MON_CAPTURE_TEST_RECOVERY_FIXTURE",JSON.stringify(seed)),seed);await page.reload();await page.waitForFunction(()=>!!window.closeFixture);}
    await page.getByRole("button",{name:"新しい録音",exact:true}).waitFor();
  };
  const clearUI=async()=>{await panel().waitFor({state:"hidden"});await page.waitForFunction(()=>{const b=[...document.querySelectorAll("button")].find(b=>b.textContent==="新しい録音");return b&&!b.disabled;});};
  try{
    await boot({...clear,phase:"checking",revision:1});await panel().waitFor();
    check(await page.getByRole("button",{name:"新しい録音",exact:true}).isDisabled()&&await retry().count()===0,"startup checking admitted capture or retry");
    await panel().getByRole("button",{name:"回復の状態を再確認",exact:true}).click();
    check(await count("retry_capture_test_recovery")===0&&await count("start_capture_test")===0&&await count("start_recording")===0,"a read replayed recovery/capture");
    await page.evaluate(()=>window.closeFixture.request(true));await page.getByRole("alert").filter({hasText:"起動時確認が進行中"}).waitFor();
    check(await count("finish_main_close")===0,"exited during startup reconciliation");results.push("startup checking blocks recording and exit; repeated reads never collect");

    await boot(attention);await retry().waitFor();check(await page.getByRole("button",{name:"新しい録音",exact:true}).isDisabled(),"attention admitted recording");
    await page.waitForTimeout(700);check(await count("retry_capture_test_recovery")===0,"attention auto-collected");
    await retry().click();await clearUI();check(await count("retry_capture_test_recovery")===1&&await page.evaluate(()=>window.closeFixture.calls.find(c=>c.command==="retry_capture_test_recovery").args.expectedRevision)===2,"retry lost exact revision");
    results.push("only explicit retry collects the exact reviewed ownership revision and releases the gate");

    await boot(attention);await page.evaluate(()=>window.closeFixture.recoveryRetryKeepsAttention=true);await retry().click();
    await page.waitForFunction(()=>window.closeFixture.captureRecovery.revision===3);await page.waitForTimeout(700);
    check(await retry().isEnabled()&&await count("retry_capture_test_recovery")===1&&await page.getByRole("button",{name:"新しい録音",exact:true}).isDisabled(),"live helper released gate or auto-retried");
    await page.evaluate(()=>window.closeFixture.recoveryRetryKeepsAttention=false);await retry().click();await clearUI();check(await count("retry_capture_test_recovery")===2,"second explicit retry was not separate");
    results.push("still-live helper preserves attention and requires another explicit retry");

    await boot();await clearUI();await page.evaluate(()=>window.closeFixture.recoveryReadFailure=true);await panel().waitFor();
    await panel().getByRole("alert").filter({hasText:"読戻し失敗"}).waitFor();check(await page.getByRole("button",{name:"新しい録音",exact:true}).isDisabled(),"unknown recovery became clear");
    await page.evaluate(()=>window.closeFixture.recoveryReadFailure=false);await panel().getByRole("button",{name:"回復の状態を再確認",exact:true}).click();await clearUI();
    check(await count("retry_capture_test_recovery")===0,"read failure triggered mutation");results.push("failed reads block recording and recover with read-only reconciliation");

    await boot(attention);await page.evaluate(()=>window.closeFixture.recoveryRetryLost=true);await retry().click();await clearUI();
    check(await count("retry_capture_test_recovery")===1,"lost receipt replayed collection");results.push("lost collection receipt is reconciled without resend");

    await boot(attention);await page.evaluate(()=>window.closeFixture.recoveryRetryHoldAck=true);await retry().click();await clearUI();
    check(await count("retry_capture_test_recovery")===1,"unending receipt replayed collection");
    await page.evaluate(()=>{window.closeFixture.captureRecovery={...window.closeFixture.captureRecovery,revision:4,phase:"attention",runId:null,errorMessage:"後続の所有状態を確認してください"};});await retry().waitFor();
    await page.evaluate(()=>window.closeFixture.recoveryPendingReplies.splice(0).forEach(resolve=>resolve()));await page.waitForTimeout(600);
    check(await panel().isVisible()&&await page.getByRole("button",{name:"新しい録音",exact:true}).isDisabled(),"old acknowledgement replaced newer ownership state");
    results.push("newer native read proof reconciles an unending receipt; a late old acknowledgement cannot clear a new attention state");

    await boot(attention);await panel().waitFor();await page.evaluate(()=>window.closeFixture.request(true));
    const closing=page.getByRole("dialog",{name:"未回収のテスト音声を保持して終了しますか？",exact:true});await closing.waitFor();
    check((await closing.innerText()).includes("強制削除")&&(await closing.innerText()).includes("所有台帳"),"exit omitted retained audio");await page.keyboard.press("Escape");await closing.waitFor({state:"hidden"});
    check(await count("finish_main_close")===0&&await count("retry_capture_test_recovery")===0,"declining exit collected or quit");
    await page.evaluate(()=>window.closeFixture.request(true));await closing.getByRole("button",{name:"音声を保持して終了へ進む",exact:true}).click();
    await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="finish_main_close"));check(await count("retry_capture_test_recovery")===0&&await page.evaluate(()=>window.closeFixture.captureRecovery.phase)==="attention","exit erased unverified audio");
    results.push("exit confirmation can decline; accepted exit retains uncollected audio and does not signal unknown processes");

    await boot();await clearUI();await page.getByRole("button",{name:"新しい録音",exact:true}).click();
    await page.getByRole("region",{name:"録音前の5秒テスト",exact:true}).waitFor();
    await page.evaluate(()=>{const f=window.closeFixture;f.captureTest={schemaVersion:1,revision:2,runId:crypto.randomUUID(),phase:"error",busy:false,microphoneUid:"built-in",microphoneName:"内蔵マイク",captureScope:{mode:"system",application:null},applicationName:null,microphone:null,system:null,cleanupPending:false,errorMessage:"模擬所有確定後の入力失敗"};f.captureRecovery={schemaVersion:1,revision:3,phase:"attention",runId:null,errorMessage:"模擬保存先の再検査が必要"};});
    await panel().waitFor();check(await retry().isDisabled(),"retry raced the current in-memory test");
    await page.getByRole("button",{name:"テスト音声を閉じる",exact:true}).click();await page.waitForFunction(()=>window.closeFixture.captureTest===null);await retry().click();await panel().waitFor({state:"hidden"});
    check(await count("discard_capture_test")===1&&await count("retry_capture_test_recovery")===1,"current failure could not be closed before startup-style retry");
    results.push("current failed test is closed first without a navigation trap before recovery retry");

    for(const [width,theme] of [[1280,"light"],[960,"dark"],[360,"dark"]]){
      await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme:theme});await boot(attention);await panel().waitFor();
      const fits=await page.evaluate(()=>{const e=document.querySelector('[aria-label="テスト音声の回復"]'),r=e.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth+1&&document.documentElement.scrollWidth<=innerWidth+1;});check(fits,`recovery overflows ${width} ${theme}`);
      await page.screenshot({path:`capture-test-recovery-${width}-${theme}.png`,fullPage:true});results.push(`recovery notice fits ${width}px ${theme}`);
    }
    return {passed:results.length,results};
  }finally{await page.context().unrouteAll();await page.emulateMedia({colorScheme:"light"});}
}
