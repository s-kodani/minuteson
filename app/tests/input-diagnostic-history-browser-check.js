// Shared production UI with synthetic IPC. Never records or plays audio.
async (page,capturePath=path=>path)=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.addInitScript(()=>{const mac=new URLSearchParams(location.search).get("resident")==="macos";Object.defineProperty(navigator,"platform",{configurable:true,get:()=>mac?"MacIntel":"Win32"});});
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const history=()=>page.getByRole("dialog",{name:"通知一覧",exact:true});
  const rows=()=>history().locator("[data-history-id]");
  const open=async()=>{await page.getByRole("button",{name:"通知一覧",exact:true}).click();await history().waitFor();};
  const quiet=async()=>check(await page.evaluate(()=>window.closeFixture.notificationHistory().length===0),"input diagnostics entered notification history");
  const waitWriter=(owner,state)=>page.waitForFunction(({owner,state})=>window.closeFixture.calls.some(c=>
    c.command==="record_input_diagnostic_log"&&c.args.stage==="live-journal-notice"&&c.args.sessionId===owner
    &&(state===null?c.args.message===null:c.args.message&&JSON.parse(c.args.message).state===state)),{owner,state});
  for(const platform of ["windows","macos"]){
    await page.setViewportSize({width:1200,height:900});
    await page.goto(`http://127.0.0.1:1420/?resident=${platform}`);await page.waitForFunction(()=>!!window.closeFixture);await page.getByRole("navigation",{name:"メインナビゲーション",exact:true}).waitFor();
    await page.evaluate(()=>{const f=window.closeFixture;f.failJournal=true;f.archived.failRead=true;return f.recordChanged({phase:"recording",sessionId:crypto.randomUUID()});});
    // Fixture construction precedes startup admission and the subscription
    // effects. Advance retry time only after both readers have actually failed.
    await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="get_archived_recording_input_notices")
      &&window.closeFixture.calls.some(c=>c.command==="record_input_diagnostic_log"&&c.args.stage==="live-journal"&&c.args.message));
    await page.clock.runFor(6200);await quiet();
    check(await page.locator(".toast-card").count()===0,"automatic journal or archive warning appeared");
    const failedReads=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_archived_recording_input_notices").length);
    check(failedReads>1,"silent failure stopped automatic retries");
    await page.evaluate(()=>window.closeFixture.failJournal=false);
    await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="record_input_diagnostic_log"&&c.args.stage==="live-journal"&&c.args.message===null));
    const logs=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="record_input_diagnostic_log"));
    check(logs.some(c=>c.args.stage==="live-journal"&&c.args.message),"silent failure stopped being logged");
    await page.evaluate(()=>{const f=window.closeFixture;f.failJournal=false;f.archived.failRead=false;
      const row=f.newArchivedRow({count:0,title:"自動再確認",historySaveIssue:"unconfirmed"});row.unavailable=true;f.archived.rows=[row];return f.recordChanged({phase:"idle"});});
    await page.clock.runFor(1200);await quiet();
    await page.evaluate(()=>{const f=window.closeFixture;const row=f.archived.rows[0];row.unavailable=false;row.historySaveIssue=null;row.revision++;return f.recordChanged({phase:"idle"});});
    await page.clock.runFor(1200);await quiet();await open();
    check(await rows().count()===0,"automatic recovery added a resolved history row");
    check(!(await history().innerText()).includes("解消済み"),"resolved label survived");
    await page.evaluate(()=>window.closeFixture.persistArchivedFixture());await page.reload();await page.waitForFunction(()=>!!window.closeFixture);await page.getByRole("navigation",{name:"メインナビゲーション",exact:true}).waitFor();await page.clock.runFor(700);await quiet();
    results.push(`${platform}: long live/archive failures, recovery and replacement WebView remain silent while retries continue`);

    const owner=await page.evaluate(()=>{const f=window.closeFixture;f.journalStatus={state:"failed",requestedRevision:2,committedRevision:1,closed:false};const owner=crypto.randomUUID();f.recordChanged({phase:"recording",sessionId:owner});return owner;});
    await waitWriter(owner,"failed");
    await open();await quiet();
    await page.evaluate(()=>window.closeFixture.journalStatus={state:"confirmed",requestedRevision:2,committedRevision:2,closed:false});
    await waitWriter(owner,null);await quiet();
    await page.evaluate(()=>window.closeFixture.journalStatus={state:"invalid",requestedRevision:3,committedRevision:2,closed:false});
    await waitWriter(owner,"invalid");await quiet();
    await page.evaluate(owner=>{const f=window.closeFixture;f.archived.rows=[f.newArchivedRow({sessionId:owner,count:0,title:"保存診断",historySaveIssue:"invalid",issue:"missing",loss:2})];return f.recordChanged({phase:"idle"});},owner);
    await page.clock.runFor(650);await quiet();
    check(await page.evaluate(()=>!window.closeFixture.calls.some(c=>c.command==="acknowledge_archived_recording_input_notice")),"silent recovery emitted a native receipt");
    results.push(`${platform}: writer failures, invalidity and recovery are logged while saved input diagnostics stay out of notifications`);

    await page.evaluate(()=>{const f=window.closeFixture;f.archived.rows=[f.newArchivedRow({title:"要音声確認",historySaveIssue:"failed",issue:"missing",loss:2})];return f.recordChanged({phase:"idle"});});
    const interruption=rows().filter({hasText:"マイクの取得が中断していました"});
    await interruption.waitFor();
    check((await interruption.innerText()).includes("保存された音声と欠損範囲を確認"),"archived interruption lost its next step");
    check(await rows().filter({hasText:/中断・再開履歴|接続通知を/u}).count()===0,"notification showed history diagnostics alongside the actual interruption");
    for(const [width,theme]of [[960,"light"],[360,"dark"]]){
      await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme:theme});
      check(await history().evaluate(e=>e.scrollWidth<=e.clientWidth+1),"notification history overflowed");
      await page.screenshot({path:capturePath(`notification-policy-${platform}-${width}-${theme}.png`)});
    }
    check(await page.evaluate(()=>!window.closeFixture.calls.some(c=>["start_recording","stop_recording","prepare_meeting_playback"].includes(c.command))),"notification observation started audio work");
    results.push(`${platform}: actual archived interruptions keep their next step in wide and narrow layouts`);
    await history().getByRole("button",{name:"通知一覧を閉じる",exact:true}).click();
    await page.setViewportSize({width:1200,height:900});
    await page.goto(`http://127.0.0.1:1420/?resident=${platform}`);await page.waitForFunction(()=>!!window.closeFixture);await page.getByRole("navigation",{name:"メインナビゲーション",exact:true}).waitFor();
    await page.evaluate(()=>{const f=window.closeFixture;f.captureInputs.microphonePermission="denied";f.captureApplications.systemPermission="permissionRequired";});
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();
    await page.locator(".toast-host .toast-error").filter({hasText:platform==="windows"?"Windowsの設定":"マイクの利用を許可"}).waitFor();
    await page.getByRole("combobox",{name:"PC音声の取得範囲",exact:true}).selectOption("application");
    await page.locator(".toast-host .toast-error").filter({hasText:platform==="windows"?"Windowsのサウンド設定":"システムオーディオ録音のみ"}).waitFor();
    check(await page.evaluate(()=>!window.closeFixture.calls.some(c=>c.command==="start_recording")),"permission diagnostics started recording");
    results.push(`${platform}: microphone and system-audio permission notices retain the OS-specific corrective instructions`);
  }
  return {passed:results.length,results};
}
