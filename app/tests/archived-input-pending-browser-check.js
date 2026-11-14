// Production scheduler with synthetic IPC; no recording, files, or playback.
async page => {
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();
    const body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const check=(value,message)=>{if(!value)throw Error(message);};
  const pending=()=>page.locator('.toast-host .toast-card').filter({hasText:'終了済み録音の通知履歴を確認中です'});
  const reads=()=>page.evaluate(()=>window.closeFixture.calls.filter(c=>['get_archived_recording_input_notices','get_archived_recording_input_notice_rows'].includes(c.command)).length);
  const event=()=>page.evaluate(()=>window.closeFixture.recordChanged({phase:'idle'}));
  await page.goto('http://127.0.0.1:1420/');await page.waitForFunction(()=>!!window.closeFixture);
  await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==='get_archived_recording_input_notices'));
  await page.clock.runFor(600);const idle=await reads();await page.clock.runFor(1600);
  check(await reads()===idle,'idle history still polls at a short interval');
  check(await pending().count()===0,'idle read displayed a pending notice');
  await page.evaluate(()=>window.closeFixture.archived.holdRead=true);await event();
  await page.waitForFunction(()=>window.closeFixture.archived.reads.length===1);const before=await reads();
  await event();await event();await page.clock.runFor(1800);
  check(await pending().count()===0,'held read displayed a pending notice');check(await reads()===before,'held read caused overlapping requests');
  await page.evaluate(()=>{const s=window.closeFixture.archived;s.holdRead=false;s.reads.splice(0).forEach(resolve=>resolve());});
  await page.waitForFunction(n=>window.closeFixture.calls.filter(c=>c.command==='get_archived_recording_input_notices').length>n,before);
  await page.evaluate(()=>{const f=window.closeFixture;f.archived.rows=[f.newArchivedRow({title:'対象だけを照合'})];});await event();
  await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==='get_archived_recording_input_notice_rows'));
  const pages=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==='get_archived_recording_input_notices').length);
  await page.clock.runFor(1200);
  check(await page.evaluate(n=>window.closeFixture.calls.filter(c=>c.command==='get_archived_recording_input_notices').length===n,pages),'active delivery rescanned all history');
  check(await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==='get_archived_recording_input_notice_rows').every(c=>c.args.targets.length===1&&c.args.targets[0].sessionId===window.closeFixture.archived.rows[0].sessionId)),'target lookup included unrelated recordings');
  await page.evaluate(()=>{window.monWindowState={visible:true,minimized:true};window.dispatchEvent(new Event('focus'));});await page.clock.runFor(300);
  const hidden=await reads();await page.clock.runFor(1000);check(await reads()===hidden,'minimized window continued polling');
  await page.evaluate(()=>{window.monWindowState={visible:true,minimized:false};window.dispatchEvent(new Event('focus'));});
  await page.waitForFunction(n=>window.closeFixture.calls.filter(c=>['get_archived_recording_input_notices','get_archived_recording_input_notice_rows'].includes(c.command)).length>n,hidden);
  await page.evaluate(()=>{
    const now=Date.now;
    Date.now=()=>now()-60_000;
    const f=window.closeFixture;
    f.archived.rows=[f.newArchivedRow({title:'時計変更後の録音'})];
    f.recordChanged({phase:'idle'});
  });
  await page.locator('.toast-host .toast-card').filter({hasText:'終了した録音「時計変更後の録音」'}).first().waitFor();
  await page.getByRole('button',{name:'通知一覧',exact:true}).click();
  const history=page.getByRole('dialog',{name:'通知一覧',exact:true});
  await history.waitFor();
  check(await page.locator('.toast-card').filter({hasText:'時計が変更されたため'}).count()===0,'wall clock rollback appeared as a notification');
  await history.getByRole('button',{name:'通知一覧を閉じる',exact:true}).click();
  const forbidden=['start_recording','stop_recording','start_capture_test','prepare_meeting_playback'];
  check(await page.evaluate(names=>window.closeFixture.calls.every(c=>!names.includes(c.command)),forbidden),'notification check invoked audio operations');
  await page.goto('http://127.0.0.1:1420/?window=recorder');await page.waitForFunction(()=>!!window.closeFixture);await page.clock.runFor(700);
  check(await reads()===0,'mini panel polled archived notices');
  return {passed:7,results:['idle histories have no short polling','hung reads stay single flight without wait toasts','events during reads trigger a follow-up','active delivery only reads targeted recordings','minimization pauses and restoration refreshes reads','wall clock rollback refreshes without a restart notification','mini panel has no history reads or audio operations']};
}
