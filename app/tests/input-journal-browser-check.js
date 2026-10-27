// CLI-first production windows with synthetic IPC; no real capture or audio.
async (page,capturePath=path=>path)=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const state=async value=>page.waitForFunction(v=>document.querySelector("[data-journal-state]")?.dataset.journalState===v,value);
  const diagnostic=()=>page.locator('.recording-journal-diagnostic:not([hidden])');
  const toast=()=>page.locator(".toast-card").filter({hasText:"入力履歴の保存:"});
  const set=async(state,requestedRevision=1000,committedRevision=999)=>page.evaluate(p=>{window.closeFixture.journalStatus={...p,closed:false};}, {state,requestedRevision,committedRevision});
  const calls=command=>page.evaluate(command=>window.closeFixture.calls.filter(c=>c.command===command).length,command);
  try{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();await page.locator("#recording-title").waitFor();
    check(await calls("get_recording_input_history_status")===0,"unowned preparation polled journal result");
    await page.getByRole("button",{name:"録音を開始",exact:true}).click();const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});
    await page.evaluate(()=>window.closeFixture.failJournal=true);
    await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});
    await page.waitForTimeout(1200);
    check(await diagnostic().count()===0&&await toast().count()===0,"helper preparation displayed an unavailable journal warning");
    await page.evaluate(()=>{const f=window.closeFixture;f.refreshInputStatus=true;f.queueState={lanes:{microphone:[],system:[]},closed:{},loss:{microphone:0,system:0}};f.inputStatus={schemaVersion:4,ownerId:f.recording.sessionId,revision:1,reportAgeMs:0,
      microphone:{state:"connected",sourceEpoch:1,interruptions:0,reason:null},system:{state:"connected",sourceEpoch:1,interruptions:0,reason:null},notices:[],noticeLoss:f.queueState.loss};return f.recordChanged({phase:"recording"});});
    await page.waitForTimeout(600);
    check(await diagnostic().count()===0&&await toast().count()===0,"one startup read failure displayed a journal warning");
    await page.evaluate(()=>window.closeFixture.failJournal=false);
    await state("confirmed");check(await diagnostic().count()===0&&await toast().count()===0,"confirmed journal created audio success/issue notice");
    await page.evaluate(()=>window.closeFixture.holdJournal=true);await page.waitForFunction(()=>window.closeFixture.pendingJournal.length===1);
    await page.waitForTimeout(3000);
    check(await toast().count()===0,"three-second diagnostic delay created a warning");
    await page.evaluate(()=>{const f=window.closeFixture;f.holdJournal=false;f.pendingJournal.splice(0).forEach(resolve=>resolve());});await state("confirmed");
    results.push("preparation and a short initial diagnostic read failure do not create a false recording warning");
    results.push("only an owned recording reads writer diagnostics; confirmation does not create an audio-success toast");
    await set("pending");await state("pending");await diagnostic().waitFor({state:'attached'});await toast().waitFor();
    check((await toast().innerText()).includes("音声の取得・保存結果とは別"),"pending journal implied audio failure/success");
    await toast().getByRole("button",{name:"注意通知を閉じる",exact:true}).click();await page.waitForTimeout(650);
    check(await toast().count()===0&&await diagnostic().count()===1,"dismissal removed pending diagnostic or heartbeat redelivered it");
    results.push("queued writes are explicit, heartbeat does not redeliver a dismissed toast, and diagnostic remains");
    await page.evaluate(()=>window.closeFixture.failJournal=true);await state("unknown");
    await page.evaluate(()=>window.closeFixture.failJournal=false);await set("confirmed",1000,1000);await state("confirmed");await page.waitForTimeout(600);
    await set("pending",1001,1000);await state("pending");await page.waitForTimeout(650);
    check(await toast().count()===0&&await diagnostic().count()===1,"short recovery or changed diagnostic resurrected a dismissed warning");
    results.push("three-second delays are quiet; short recovery and diagnostic changes retain the dismissed occurrence");
    await set("failed",1001,1000);await state("failed");await toast().waitFor();
    check(await page.locator('[data-input-kind="microphone"]').getAttribute("data-input-state")==="connected","history write failure changed live acquisition");
    const id=await toast().getAttribute("data-toast-id");await page.waitForTimeout(650);check(await toast().getAttribute("data-toast-id")===id,"failed diagnostic duplicated by heartbeat");
    results.push("writer failure remains distinct from connected input and is not duplicated by heartbeat");
    for(const theme of ["light","dark"]){await page.emulateMedia({colorScheme:theme});for(const width of [960,360]){
      await page.setViewportSize({width,height:900});await page.waitForTimeout(150);
      check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`journal diagnostic overflow ${width} ${theme}`);
      await page.screenshot({path:capturePath(`journal-save-main-${theme}-${width}.png`)});
    }}results.push("failed journal fits 960/360px light/dark layouts without an inline diagnosis banner");
    await page.evaluate(()=>window.closeFixture.failJournal=true);await state("unknown");
    check(await toast().getAttribute("data-toast-id")===id,"read failure replaced the unresolved writer failure");
    await page.evaluate(()=>window.closeFixture.failJournal=false);await set("confirmed",1001,1001);await state("confirmed");
    await page.waitForFunction(()=>![...document.querySelectorAll('.toast-card')].some(e=>e.textContent.includes('入力履歴の保存:')));
    check(await diagnostic().count()===0&&await toast().count()===0,"fresh writer confirmation did not resolve diagnostic");
    results.push("unreadable diagnostics retain previous failure; only fresh writer confirmation clears it");
    await page.evaluate(()=>window.closeFixture.holdJournal=true);await page.waitForFunction(()=>window.closeFixture.pendingJournal.length===1);const held=await calls("get_recording_input_history_status"),reads=await calls("get_recording_input_status");
    await state("stale");await page.waitForTimeout(600);check(await calls("get_recording_input_history_status")===held,"hung journal poll overlapped");
    check(await calls("get_recording_input_status")>reads,"hung journal stopped independent input polling");
    results.push("hung journal read is single-flight and ages out while input acquisition reads continue");
    await page.evaluate(()=>{const f=window.closeFixture;f.journalStatus={state:"failed",requestedRevision:2,committedRevision:1,closed:false};return f.recordChanged({sessionId:crypto.randomUUID()});});
    await page.evaluate(()=>{const f=window.closeFixture;f.holdJournal=false;f.pendingJournal.splice(0).forEach(resolve=>resolve());});
    await state("failed");await page.waitForTimeout(650);check(await diagnostic().count()===1,"late old-owner reply erased new-owner failure");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"saved"}));await state("inactive");const stopped=await calls("get_recording_input_history_status");await page.waitForTimeout(650);
    check(await calls("get_recording_input_history_status")===stopped&&await diagnostic().count()===0&&await toast().count()===0,"terminal owner retained live diagnostic/poll");
    results.push("late replies cannot cross recording owners; terminal recording stops live diagnostics and polling");
    await page.evaluate(()=>{const f=window.closeFixture;f.archived.rows=[f.newArchivedRow({title:"停止時の保存診断",legacy:true,historySaveIssue:"unconfirmed"})];return f.recordChanged({phase:"idle"});});
    await page.getByRole('button',{name:'通知一覧',exact:true}).click();
    const history=()=>page.getByRole('dialog',{name:'通知一覧',exact:true});
    const archived=history().locator('.toast-card').filter({hasText:'停止時の取得履歴保存が未確定'});await archived.waitFor();
    check((await archived.innerText()).includes("停止時の取得履歴保存が未確定"),"missing delivery queue hid final writer uncertainty");
    await page.evaluate(()=>window.closeFixture.persistArchivedFixture());await page.reload();await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole('button',{name:'通知一覧',exact:true}).click();await archived.waitFor();check((await archived.innerText()).includes("停止時の取得履歴保存が未確定"),"restart lost final manifest diagnosis");
    results.push("final writer uncertainty persists across replacement WebView even without a notification queue");
    await page.goto("http://127.0.0.1:1420/?window=recorder");await page.waitForFunction(()=>!!window.closeFixture);await page.setViewportSize({width:227,height:101});
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording",sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID()}));
    await set("failed",2,1);await page.waitForTimeout(400);
    for(const theme of ["light","dark"]){await page.emulateMedia({colorScheme:theme});await page.waitForTimeout(150);
      check(await page.locator(".mini-shell").evaluate(e=>e.scrollWidth<=e.clientWidth+1&&e.scrollHeight<=e.clientHeight+1),"mini history diagnostic overflowed native geometry");
      await page.screenshot({path:capturePath(`journal-save-mini-${theme}-227x101.png`)});
    }
    check(await page.locator(".mini-journal-status").count()===0,"mini retained journal diagnostic text");
    results.push("mini remains icon-only at 227x101 in both themes even when journal persistence fails");
    await set("invalid",3,1);await page.waitForTimeout(400);await set("confirmed",3,3);await page.waitForTimeout(400);
    check(await page.locator(".mini-journal-status").count()===0,"mini showed an invalid journal diagnostic");
    results.push("invalid and contradictory writer reports add no text to the mini");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`);}
  finally{await page.context().unrouteAll();await page.evaluate(()=>sessionStorage.removeItem("MON_ARCHIVED_INPUT_FIXTURE")).catch(()=>{});}
}
