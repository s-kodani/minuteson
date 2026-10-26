// Production history notices with synthetic IPC; no audio or native capture.
async page => {
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();
    const body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const check=(value,message)=>{if(!value)throw Error(message);};
  const pending=()=>page.locator('.toast-host .toast-card').filter({hasText:'終了済み録音の通知履歴を確認中です'});
  const reads=()=>page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==='get_archived_recording_input_notices').length);
  await page.goto('http://127.0.0.1:1420/');
  await page.waitForFunction(()=>!!window.closeFixture);
  await page.waitForTimeout(2300);
  check(await pending().count()===0,'responsive reads displayed a pending notice');
  await page.evaluate(()=>window.closeFixture.archived.holdRead=true);
  await page.waitForFunction(()=>window.closeFixture.archived.reads.length===1);
  const before=await reads();
  await page.waitForTimeout(1800);
  check(await pending().count()===1,'held read did not display exactly one pending notice');
  check(await reads()===before,'held read caused overlapping requests');
  await page.evaluate(()=>{
    const state=window.closeFixture.archived;
    state.holdRead=false;
    state.reads.splice(0).forEach(resolve=>resolve());
  });
  await pending().waitFor({state:'detached'});
  await page.waitForTimeout(2300);
  check(await pending().count()===0,'completed read repeatedly displayed a pending notice');
  check(await reads()>before,'polling did not resume after held read completed');
  const forbidden=['start_recording','stop_recording','start_capture_test','prepare_meeting_playback'];
  check(await page.evaluate(names=>window.closeFixture.calls.every(c=>!names.includes(c.command)),forbidden),'notification test invoked audio operations');
  await page.goto('http://127.0.0.1:1420/?window=recorder');
  await page.waitForFunction(()=>!!window.closeFixture);
  await page.waitForTimeout(700);
  check(await reads()===0,'mini panel polled archived notices');
  return {passed:4,results:['responsive reads do not display pending notices','hung reads show one pending notice without duplicate requests','completion clears pending and resumes polling without repeated notices','mini panel does not own archived reads; no recording or playback commands']};
}
