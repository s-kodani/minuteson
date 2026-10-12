// Native delivery protocol is synthetic; no recording, playback or files.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
  await page.evaluate(()=>{const f=window.closeFixture;f.archived.rows=[f.newArchivedRow({title:"通知一覧の点検会議",both:true,loss:2,issue:"missing"})];return f.recordChanged({phase:"idle"});});
  await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="acknowledge_archived_recording_input_notice"));
  await page.waitForTimeout(600);
  check(await page.locator(".archived-input-notices,.issue-status").count()===0,"archive banner escaped workspace");
  check(await page.locator(".toast-host").count()===1,"Sonner presenter missing");
  results.push("archived input events reach Sonner while retained diagnostics have no workspace banner");
  await page.getByRole("button",{name:"通知一覧",exact:true}).click();
  const dialog=page.getByRole("dialog",{name:"通知一覧",exact:true});await dialog.waitFor();
  await dialog.getByText(/マイク通知の保持不足 2件/).first().waitFor();
  await page.waitForFunction(()=>Object.values(window.closeFixture.archived.rows[0].lanes).every(l=>l[0].delivery.rendered));
  const diagnostic=dialog.locator(".toast-card").filter({hasText:"マイク通知の保持不足 2件"});
  check((await diagnostic.innerText()).includes("取得履歴がありません"),"historical diagnostics missing from list");
  results.push("actual toast/list visibility acknowledges delivery; retained history and losses appear only in the list");
  const mic=dialog.locator(".toast-card").filter({hasText:"マイクの取得が中断していました"});
  await mic.getByRole("button",{name:"注意通知を閉じる",exact:true}).click();
  await dialog.locator(".toast-card").filter({hasText:"マイクの取得が復帰していました"}).waitFor();
  check(await page.evaluate(()=>window.closeFixture.archived.rows[0].lanes.system[0].delivery.sequence)===1,"microphone dismissal advanced another lane");
  check(await diagnostic.count()===1,"closing delivery deleted retained diagnosis");
  results.push("closing one input advances only its exact lane and does not delete recording diagnostics");
  await diagnostic.getByRole("button",{name:"会議を開く",exact:true}).click();await dialog.waitFor({state:"hidden"});
  await page.getByRole("tab",{name:"文字起こし",exact:true}).waitFor();
  check(await page.locator(".meeting-detail-status h1").innerText()==="通知一覧の点検会議","notification opened wrong meeting");
  check(await page.locator(".archived-input-notices,.issue-status").count()===0,"navigation recreated inline notifications");
  check(await page.evaluate(()=>!window.closeFixture.calls.some(c=>["start_recording","start_capture_test","prepare_meeting_playback","prepare_utterance_playback"].includes(c.command))),"notification action started recording or playback");
  results.push("meeting action closes the list and opens the exact meeting without recording/playback");
  return {passed:results.length,results};
}
