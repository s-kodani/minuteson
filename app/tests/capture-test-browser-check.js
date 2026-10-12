// Production MainWindow with synthetic IPC and silent PCM only. No native capture.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const panel=()=>page.getByRole("region",{name:"録音前の5秒テスト",exact:true});
  const start=()=>panel().getByRole("button",{name:"5秒録音して確認",exact:true});
  const modal=()=>page.getByRole("dialog",{name:"5秒の録音前テストを開始しますか？",exact:true});
  const accept=()=>modal().getByRole("button",{name:"5秒テストを開始する",exact:true});
  const mic=()=>page.getByRole("combobox",{name:"録音するマイク",exact:true});
  const normal=()=>page.getByRole("region",{name:"録音状態",exact:true}).getByRole("button",{name:"録音を開始",exact:true});
  const count=cmd=>page.evaluate(cmd=>window.closeFixture.calls.filter(c=>c.command===cmd).length,cmd);
  const idle=()=>page.waitForFunction(()=>{const b=[...document.querySelectorAll('.capture-test-panel button')].find(b=>b.textContent==="5秒録音して確認");return b&&!b.disabled;});
  const boot=async()=>{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();await panel().waitFor();await idle();
  };
  const begin=async()=>{await start().click();await accept().click();await modal().waitFor({state:"hidden"});await panel().getByText("テストの入力を初期化しています",{exact:true}).waitFor();};
  const ready=async()=>{await page.evaluate(()=>window.closeFixture.testReady());await panel().getByText("停止と音声の検証が完了しました",{exact:true}).waitFor();};
  const play=kind=>panel().getByRole("button",{name:`${kind}のテスト音声を再生`,exact:true});
  const discard=()=>panel().getByRole("button",{name:"テスト音声を閉じる",exact:true});
  const released=()=>page.waitForFunction(()=>Object.keys(window.closeFixture.testPlayback.tokens).length===0&&Object.keys(window.closeFixture.testPlayback.reservations).length===0);
  try{
    await boot();
    check(await normal().isEnabled()&&await count("start_capture_test")===0&&await count("start_recording")===0&&await count("create_meeting")===0&&await count("get_capture_test_levels")===0,"opening preparation started capture or required test");
    results.push("preparation reads test ownership only, never starts capture, creates meetings or requires a test before recording");
    await start().click();await modal().waitFor();
    check((await modal().innerText()).includes("内蔵マイク")&&(await modal().innerText()).includes("通知音")&&await mic().isDisabled()&&await count("start_capture_test")===0,"confirmation omitted scope or started early");
    await page.keyboard.press("Escape");await modal().waitFor({state:"hidden"});await idle();
    check(await count("start_capture_test")===0&&await page.evaluate(()=>window.dataFixture.meetings.length===1),"decline started or saved meeting");
    results.push("explicit confirmation freezes microphone/scope and explains private five-second audio; declining has no capture or DB effect");
    await start().click();await accept().dblclick();await modal().waitFor({state:"hidden"});
    await panel().getByRole("button",{name:"テストを取り消す",exact:true}).waitFor();
    const request=await page.evaluate(()=>window.closeFixture.calls.find(c=>c.command==="start_capture_test").args.request);
    check(await count("start_capture_test")===1&&request.microphoneUid==="built-in"&&request.captureScope.mode==="system"&&request.expectedImport.token===null&&await count("start_recording")===0&&await count("create_meeting")===0&&await page.evaluate(()=>window.dataFixture.meetings.length===1),"test confirmation did not reserve exact independent ownership once");
    check(await mic().isDisabled()&&await normal().isDisabled()&&await start().isDisabled(),"busy test allowed competing input/capture");
    results.push("double consent sends one fixed run/input/import request without a meeting and blocks competing recording or input changes");
    await page.evaluate(()=>{const f=window.closeFixture;f.testChanged({phase:"recording"});f.testLevels={schemaVersion:1,ownerId:f.captureTest.runId,revision:1,reportAgeMs:0,microphone:{state:"measured",windowFrames:1024,ageMs:0,rms:0.25,peak:0.5},system:{state:"waiting",windowFrames:null,ageMs:null,rms:null,peak:null}};});
    await panel().getByRole("meter",{name:"テストのマイクのピーク入力レベル",exact:true}).waitFor();
    check(await panel().getByRole("meter").getAttribute("value")==="0.5"&&await count("get_recording_levels")===0,"test meters adopted session telemetry or wrong PCM");
    results.push("busy test shows run-owned separate PCM meters, never reads normal recording telemetry");
    await ready();check(await count("prepare_capture_test_playback")===0&&await count("read_playback_chunk")===0,"ready autoplayed");
    check((await panel().innerText()).includes("5.000秒")&&(await panel().innerText()).includes("4.000秒")&&(await panel().innerText()).includes("欠損あり"),"received physical durations or gaps hidden");
    results.push("verified readiness never autoplays and displays per-track physical duration/gaps without filling missing audio");
    await play("マイク").click();await panel().getByRole("button",{name:"一時停止",exact:true}).waitFor();
    await play("PC").click();await page.waitForFunction(()=>window.closeFixture.calls.filter(c=>c.command==="prepare_capture_test_playback").length===2);await panel().getByRole("button",{name:"一時停止",exact:true}).waitFor();
    const playback=await page.evaluate(()=>window.closeFixture.calls.filter(c=>["register_playback_preparation","prepare_capture_test_playback","finish_playback_preparation"].includes(c.command)));
    check(playback.length===6&&playback.filter(c=>c.command==="prepare_capture_test_playback").map(c=>c.args.trackKind).join(",")==="mic,system"&&playback.every(c=>c.args.preparationId)&&await count("close_playback")>=1,"separate playback ownership or prior-token close missing");
    results.push("explicit mic and PC playback register/prepare/acknowledge separately; switching closes the prior private token");
    await mic().selectOption("usb-123");
    await panel().getByRole("alert").filter({hasText:"現在の入力選択とは異なる"}).waitFor();await released();
    check(await play("マイク").isDisabled()&&await play("PC").isDisabled()&&(await panel().innerText()).includes("テスト時のマイク: 内蔵マイク"),"old test was relabelled or played as new input");
    results.push("changing microphone keeps frozen input labels, stops old playback and disables stale-result playback");
    await mic().selectOption("built-in");await play("マイク").click();await panel().getByRole("button",{name:"一時停止",exact:true}).waitFor();
    await discard().click();await released();await idle();
    check(await page.evaluate(()=>window.closeFixture.captureTest===null)&&await count("discard_capture_test")===1&&await count("start_recording")===0,"explicit discard leaked or started recording");
    results.push("closing test stops playback, waits for exact token cleanup and discards only test audio");

    await boot();await begin();await panel().getByRole("button",{name:"テストを取り消す",exact:true}).click();
    await panel().getByText("テストの停止完了を待っています",{exact:true}).waitFor();
    check(await normal().isDisabled()&&await discard().count()===0&&await count("cancel_capture_test")===1,"cancel receipt falsely meant stopped");
    await page.evaluate(()=>window.closeFixture.testChanged({phase:"cancelled",busy:false}));await discard().waitFor();await discard().click();await idle();
    results.push("cancellation remains busy until native termination; terminal cancelled ownership is explicitly discarded before reuse");

    await boot();await page.evaluate(()=>window.closeFixture.testLost=true);await start().click();await accept().click();
    await modal().getByRole("alert").waitFor();await modal().getByRole("button",{name:"確認を閉じる",exact:true}).click();await modal().waitFor({state:"hidden"});
    await panel().getByText("テストの入力を初期化しています",{exact:true}).waitFor();
    check(await count("start_capture_test")===1&&await normal().isDisabled(),"lost accepted reply replayed or enabled capture");
    const firstPoll=await count("get_capture_test_snapshot");await page.waitForFunction(n=>window.closeFixture.calls.filter(c=>c.command==="get_capture_test_snapshot").length>n,firstPoll);check(await count("start_capture_test")===1,"read replayed start");
    results.push("lost accepted start reply is reconciled by read-only ownership polling, never by resubmitting capture");

    await boot();await page.evaluate(()=>window.closeFixture.testAbsentStart=true);await start().click();await accept().click();
    await modal().getByRole("alert").waitFor();await modal().getByRole("button",{name:"確認を閉じる",exact:true}).click();await modal().waitFor({state:"hidden"});
    await panel().getByRole("alert").filter({hasText:"開始要求の受付結果が未確認"}).waitFor();
    const absentPoll=await count("get_capture_test_snapshot");await page.waitForFunction(n=>window.closeFixture.calls.filter(c=>c.command==="get_capture_test_snapshot").length>n,absentPoll);
    check(await count("start_capture_test")===1&&await start().isDisabled()&&await normal().isDisabled(),"unknown absence allowed replay");
    results.push("missing result after an uncertain start stays unknown and blocks new recording without guessing that audio stopped");

    await boot();await page.evaluate(()=>window.closeFixture.failFlush=true);await start().click();await accept().click();await modal().getByRole("alert").waitFor();
    check(await count("start_capture_test")===0,"failed draft save started test");await modal().getByRole("button",{name:"確認を閉じる",exact:true}).click();
    results.push("failed pending-edit save prevents test admission and retains existing meetings/input");

    await boot();await begin();await ready();await page.evaluate(()=>window.closeFixture.testDiscardFailure=true);await discard().click();
    await page.getByRole("alert").filter({hasText:"模擬テスト回収未完了"}).first().waitFor();
    await normal().click();const n=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});await n.getByRole("button",{name:"録音を開始する",exact:true}).click();await n.getByRole("alert").waitFor();
    check(await count("start_recording")===0&&await page.evaluate(()=>window.dataFixture.meetings.length===1),"failed discard saved normal meeting or started capture");await n.getByRole("button",{name:"確認を閉じる",exact:true}).click();
    results.push("failed private-audio collection prevents normal recording and meeting creation even after a second confirmation");

    await boot();await begin();await ready();await play("マイク").click();await panel().getByRole("button",{name:"一時停止",exact:true}).waitFor();
    await page.evaluate(()=>window.closeFixture.testPlayback.failClose=true);await discard().click();await page.getByRole("alert").filter({hasText:"模擬テスト再生解放失敗"}).first().waitFor();
    check(await count("discard_capture_test")===0&&await page.evaluate(()=>Object.keys(window.closeFixture.testPlayback.tokens).length===1),"failed playback release dropped ownership");
    await page.evaluate(()=>window.closeFixture.testPlayback.failClose=false);await discard().click();await released();await idle();
    results.push("failed known-token release retains ownership and blocks discard until explicit cleanup retry succeeds");

    await boot();await begin();await ready();await page.evaluate(()=>window.closeFixture.testPlayback.hold=true);await play("PC").click();await page.waitForFunction(()=>window.closeFixture.testPlayback.pending.length===1);
    await panel().getByRole("button",{name:"テスト音声の再生準備を取り消す",exact:true}).click();await discard().click();
    await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="cancel_playback_preparation"));
    check(await count("discard_capture_test")===0,"cancel receipt dropped pending playback worker");
    await page.evaluate(()=>{const p=window.closeFixture.testPlayback;p.hold=false;p.pending.splice(0).forEach(resolve=>resolve());});await released();await idle();
    check(await count("read_playback_chunk")===0&&await count("discard_capture_test")===1,"late cancelled worker revived test playback");
    results.push("test discard waits for an actually terminating playback preparation, not just its cancellation receipt");

    await boot();await begin();await ready();await normal().click();
    check((await n.innerText()).includes("一時音声を破棄"),"normal confirmation omitted test disposal");
    await n.getByRole("button",{name:"録音を開始する",exact:true}).click();await n.waitFor({state:"hidden"});
    const order=await page.evaluate(()=>window.closeFixture.calls.filter(c=>["discard_capture_test","start_recording"].includes(c.command)).map(c=>c.command).join(","));
    check(order==="discard_capture_test,start_recording"&&await count("start_recording")===1&&await page.evaluate(()=>window.dataFixture.meetings.length===2),"normal capture skipped owned test collection");
    results.push("normal recording collects terminal test audio before one confirmed meeting/capture admission");

    await boot();await begin();await page.evaluate(()=>window.closeFixture.request(true));
    const close=page.getByRole("dialog",{name:"録音前テストを閉じて終了しますか？",exact:true});await close.waitFor();await page.keyboard.press("Escape");await close.waitFor({state:"hidden"});
    check(await count("cancel_capture_test")===0&&await count("finish_main_close")===0,"declining close cancelled or exited");
    await page.evaluate(()=>window.closeFixture.request(true));await close.getByRole("button",{name:"テストを閉じて終了へ進む",exact:true}).click();
    await page.waitForFunction(()=>window.closeFixture.captureTest?.phase==="cancelling");
    check(await count("finish_main_close")===0&&await count("discard_capture_test")===0,"close raced process termination");
    await page.evaluate(()=>window.closeFixture.testChanged({phase:"cancelled",busy:false}));await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="finish_main_close"));
    check(await count("cancel_capture_test")===1&&await count("discard_capture_test")===1,"close did not cancel/collect exactly owned test");
    results.push("close confirmation can decline safely; accepted exit waits for test termination and collection before final native guard");

    const details=()=>page.getByRole("dialog",{name:"録音の詳細設定",exact:true});
    const openDetails=async()=>{await page.getByRole("button",{name:"録音の詳細設定",exact:true}).click();await details().waitFor();};
    const closeDetails=async()=>{await page.keyboard.press("Escape");await details().waitFor({state:"hidden"});};
    await boot();await openDetails();
    await page.getByRole("combobox",{name:"PC音声の取得範囲",exact:true}).selectOption("application");
    const application=page.getByRole("combobox",{name:"録音するアプリ",exact:true});await application.waitFor();
    await page.waitForFunction(()=>{const e=document.querySelector('[aria-label="録音するアプリ"]');return e&&!e.disabled;});
    check(await start().isDisabled(),"unselected app permitted test");
    const appKey=await application.locator('option').nth(1).getAttribute("value");await application.selectOption(appKey);
    await closeDetails();
    await start().click();check((await modal().innerText()).includes("会議ブラウザ")&&(await modal().innerText()).includes("他のタブ"),"app confirmation omitted scope limit");
    await accept().click();await modal().waitFor({state:"hidden"});await ready();
    const scoped=await page.evaluate(()=>window.closeFixture.calls.find(c=>c.command==="start_capture_test").args.request.captureScope);
    check(scoped.mode==="application"&&scoped.application.pid===123&&scoped.application.launchTimeUs===1791100000000001&&(await panel().innerText()).includes("会議ブラウザ"),"app test lost exact process identity");
    await openDetails();await page.getByRole("combobox",{name:"PC音声の取得範囲",exact:true}).selectOption("system");await closeDetails();await panel().getByRole("alert").filter({hasText:"現在の入力選択とは異なる"}).waitFor();
    check(await play("PC").isDisabled()&&(await panel().innerText()).includes("会議ブラウザ"),"old application test relabelled as system");
    results.push("app test requires explicit process selection, freezes exact identity and tab limitations, and never relabels its result after scope changes");

    await boot();await openDetails();await page.getByRole("combobox",{name:"PC音声の取得範囲",exact:true}).selectOption("application");
    await page.waitForFunction(()=>{const e=document.querySelector('[aria-label="録音するアプリ"]');return e&&!e.disabled;});
    await application.selectOption(await application.locator('option').nth(1).getAttribute("value"));
    await closeDetails();
    await page.evaluate(()=>window.closeFixture.mutateApplicationDuringFlush=true);await start().click();await accept().click();await modal().getByRole("alert").waitFor();
    check(await count("start_capture_test")===0,"restarted app accepted test");await modal().getByRole("button",{name:"確認を閉じる",exact:true}).click();
    results.push("application restart during edit flushing rejects test before admission without falling back to whole-system capture");

    await boot();await page.evaluate(()=>window.closeFixture.captureInputs.microphonePermission="denied");await start().click();await page.getByRole("alert").filter({hasText:"選択したマイクまたは権限"}).first().waitFor();
    check(await count("start_capture_test")===0&&await modal().count()===0,"denied microphone prompted/started test");
    results.push("fresh denied microphone permission prevents test admission without choosing another device");

    for(const property of ["losePrepare","loseReceipt"]){
      await boot();await begin();await ready();await page.evaluate(property=>window.closeFixture.testPlayback[property]=true,property);await play("マイク").click();await panel().getByRole("alert").filter({hasText:"音声を再生できません"}).waitFor();await released();
      check(await count("read_playback_chunk")===0&&await count("cancel_playback_preparation")>=1,"lost playback reply produced audio or leaked reservation");
      await discard().click();await idle();results.push(`${property}: lost private playback reply releases reservation/token without scheduling PCM`);
    }

    await boot();await begin();await ready();await play("マイク").click();await panel().getByRole("button",{name:"一時停止",exact:true}).waitFor();await page.evaluate(()=>window.closeFixture.testReadFailure=true);
    await panel().getByText("テストの状態を確認できていません。",{exact:true}).waitFor();await released();
    check(await play("マイク").isDisabled()&&await normal().isDisabled()&&await start().isDisabled(),"unknown ownership allowed playback/capture");
    await page.evaluate(()=>window.closeFixture.testReadFailure=false);await idle();
    check(await count("start_capture_test")===1&&await play("マイク").isEnabled(),"read recovery resent capture");
    results.push("ownership read failure stops and locks private playback/recording; valid read recovery never restarts capture");

    await boot();await begin();await ready();await page.evaluate(()=>window.closeFixture.testDiscardLost=true);await discard().click();await idle();
    check(await page.evaluate(()=>window.closeFixture.captureTest===null)&&await count("discard_capture_test")===1,"lost collection reply replayed disposal");
    results.push("lost discard receipt is accepted only after authoritative empty ownership, without replaying collection");

    await boot();await begin();await ready();await start().click();check((await modal().innerText()).includes("前のテスト音声"),"new test omitted disposal");await accept().click();await modal().waitFor({state:"hidden"});
    const restarts=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="start_capture_test").map(c=>c.args.request.runId));
    check(restarts.length===2&&restarts[0]!==restarts[1]&&await count("discard_capture_test")===1&&await page.evaluate(()=>window.dataFixture.meetings.length===1),"new test replayed owner or saved meeting");
    results.push("a new confirmed test collects previous terminal audio and uses a fresh run ID without meeting publication");

    await boot();await begin();await ready();
    for(const width of [1280,960,360])for(const theme of ["light","dark"]){
      await page.setViewportSize({width,height:700});await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
      await page.waitForFunction(t=>document.documentElement.dataset.theme===t,theme);await play("マイク").scrollIntoViewIfNeeded();
      check(await panel().evaluate(e=>e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth),"test panel horizontal overflow");
      await page.screenshot({path:`output/playwright/test-controls-${theme}-${width}-20261012.png`});
    }
    results.push("ready inputs, separate playback and cleanup controls fit both themes at 1280/960/360px without horizontal overflow");
    await discard().click();await idle();return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases (${results.at(-1)??"setup"}): ${cause}; body=${await page.locator('body').innerText()}`);}
}
