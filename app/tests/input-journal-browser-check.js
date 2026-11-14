// CLI-first production windows with synthetic IPC; no real capture or audio.
async (page,capturePath=path=>path)=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/u,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const set=(state,requestedRevision,committedRevision)=>page.evaluate(p=>{window.closeFixture.journalStatus={...p,closed:false};},{state,requestedRevision,committedRevision});
  const calls=command=>page.evaluate(command=>window.closeFixture.calls.filter(c=>c.command===command).length,command);
  const writerLogs=()=>page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="record_input_diagnostic_log"&&c.args.stage==="live-journal-notice").map(c=>c.args));
  const waitLog=(stage,message)=>page.waitForFunction(({stage,message})=>window.closeFixture.calls.some(c=>
    c.command==="record_input_diagnostic_log"&&c.args.stage===stage&&(message===null?c.args.message===null:c.args.message?.includes(message))),{stage,message});
  const quiet=async()=>{
    check(await page.locator(".toast-card,.recording-journal-diagnostic,.mini-journal-status").count()===0,"journal diagnostics reached the workspace");
    check(await page.evaluate(()=>window.closeFixture.notificationHistory().length===0),"journal diagnostics entered the notification list");
  };
  try{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();await page.locator("#recording-title").waitFor();
    check(await calls("get_recording_input_history_status")===0,"unowned preparation polled journal result");
    await page.getByRole("button",{name:"録音を開始",exact:true}).click();
    const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});
    await page.evaluate(()=>window.closeFixture.failJournal=true);
    await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});
    await waitLog("live-journal","模擬履歴診断読込失敗");await quiet();
    await page.evaluate(()=>{const f=window.closeFixture;f.refreshInputStatus=true;f.queueState={lanes:{microphone:[],system:[]},closed:{},loss:{microphone:0,system:0}};f.inputStatus={schemaVersion:4,ownerId:f.recording.sessionId,revision:1,reportAgeMs:0,
      microphone:{state:"connected",sourceEpoch:1,interruptions:0,reason:null},system:{state:"connected",sourceEpoch:1,interruptions:0,reason:null},notices:[],noticeLoss:f.queueState.loss};return f.recordChanged({phase:"recording"});});
    await page.evaluate(()=>window.closeFixture.failJournal=false);
    await waitLog("live-journal",null);await quiet();
    results.push("only owned recordings read journal status; read failure and recovery stay internal");

    await set("pending",1000,999);await page.clock.runFor(2500);await quiet();
    check((await writerLogs()).length===0,"pending journal was logged as a write failure");
    await set("failed",1001,1000);await waitLog("live-journal-notice",'"state":"failed"');await quiet();
    check(await page.locator('[data-input-kind="microphone"]').getAttribute("data-input-state")==="connected","journal failure changed live acquisition");
    await page.clock.runFor(650);check((await writerLogs()).length===1,"heartbeat duplicated a confirmed writer failure");
    results.push("writer failure stays internal and is logged once while input is connected");

    await page.evaluate(()=>window.closeFixture.holdJournal=true);
    await page.waitForFunction(()=>window.closeFixture.pendingJournal.length===1);
    const held=await calls("get_recording_input_history_status"),reads=await calls("get_recording_input_status");
    await page.clock.runFor(1600);await quiet();
    check(await calls("get_recording_input_history_status")===held,"hung journal poll overlapped");
    check(await calls("get_recording_input_status")>reads,"hung journal stopped independent input polling");
    await page.evaluate(()=>{const f=window.closeFixture;f.holdJournal=false;f.pendingJournal.splice(0).forEach(resolve=>resolve());});
    check((await writerLogs()).length===1,"unreadable journal replaced a confirmed failure");
    await set("confirmed",1001,1001);await waitLog("live-journal-notice",null);await quiet();
    results.push("hung journal reads leave input polling independent; confirmed recovery clears the writer failure log");

    const originalOwner=await page.evaluate(()=>{
      const f=window.closeFixture;f.holdJournal=true;f.journalStatus={state:"failed",requestedRevision:1002,committedRevision:1001,closed:false};
      return f.recording.sessionId;
    });
    await page.waitForFunction(()=>window.closeFixture.pendingJournal.length===1);
    await page.evaluate(()=>{
      const f=window.closeFixture;f.journalStatus={state:"invalid",requestedRevision:2,committedRevision:1,closed:false};
      return f.recordChanged({sessionId:crypto.randomUUID()});
    });
    await page.evaluate(()=>{const f=window.closeFixture;f.holdJournal=false;f.pendingJournal.splice(0).forEach(resolve=>resolve());});
    await waitLog("live-journal-notice",'"state":"invalid"');await quiet();
    check((await writerLogs()).filter(c=>c.sessionId===originalOwner).length===2,"late reply added a writer failure for the old recording");
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"saved"}));
    await page.clock.runFor(350);const stopped=await calls("get_recording_input_history_status");await page.clock.runFor(650);
    check(await calls("get_recording_input_history_status")===stopped,"terminal owner retained live journal polling");
    results.push("late replies cannot cross recording owners; terminal recording stops live polling");

    await page.goto("http://127.0.0.1:1420/?window=recorder");await page.waitForFunction(()=>!!window.closeFixture);await page.setViewportSize({width:227,height:101});
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording",sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID()}));
    await set("failed",2,1);await waitLog("live-journal-notice",'"state":"failed"');await quiet();
    for(const theme of ["light","dark"]){
      await page.emulateMedia({colorScheme:theme});await page.clock.runFor(150);
      check(await page.locator(".mini-shell").evaluate(e=>e.scrollWidth<=e.clientWidth+1&&e.scrollHeight<=e.clientHeight+1),"mini history diagnostic overflowed native geometry");
      await page.screenshot({path:capturePath("journal-save-mini-"+theme+"-227x101.png")});
    }
    await set("invalid",3,1);await waitLog("live-journal-notice",'"state":"invalid"');await quiet();
    results.push("mini reports writer failure and invalidity internally and stays within its native geometry");
    return {passed:results.length,results};
  }catch(cause){throw Error("after "+results.length+" cases: "+cause+"; body="+await page.locator("body").innerText());}
  finally{await page.context().unrouteAll();}
}
