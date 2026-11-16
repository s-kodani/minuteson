// Production main window with synthetic recovery and closed-input events.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  try{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("navigation",{name:"メインナビゲーション",exact:true}).waitFor();
    await page.evaluate(()=>{const f=window.closeFixture;f.archived.rows=[f.newArchivedRow({title:"通知を出さない会議",both:true})];return f.recordChanged({phase:"idle"});});
    await page.clock.runFor(1600);
    check(await page.evaluate(()=>window.closeFixture.notificationHistory().length===0),"closed input events created notifications");
    check(await page.evaluate(()=>window.closeFixture.calls.every(c=>!c.command.includes("archived_recording_input"))),"retired input notification delivery still runs");
    results.push("closed microphone and PC interruption/recovery events neither notify nor run delivery IO");
    const before=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="list_meetings").length);
    await page.evaluate(()=>window.emitResidencyFixture("recordings-recovered",[window.closeFixture.archived.rows[0].meetingId]));
    await page.waitForFunction(n=>window.closeFixture.calls.filter(c=>c.command==="list_meetings").length>n,before);
    check(await page.evaluate(()=>window.closeFixture.notificationHistory().length===0),"successful recovery created a notification");
    results.push("successful recovery refreshes meetings silently");
    await page.evaluate(()=>window.closeFixture.showQueueBlockers());
    await page.getByRole("button",{name:"通知一覧",exact:true}).click();
    const dialog=page.getByRole("dialog",{name:"通知一覧",exact:true});await dialog.waitFor();
    await dialog.locator(".toast-card").filter({hasText:"合成優先通知 block1"}).waitFor();
    check(!(await dialog.innerText()).match(/取得が中断|取得が復帰|前回の音声保存/u),"removed input/recovery notification appeared in the list");
    results.push("other error notifications remain available in the notification list");
    return {passed:results.length,results};
  }finally{await page.context().unrouteAll();}
}
