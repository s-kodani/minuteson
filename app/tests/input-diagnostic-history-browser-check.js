// Production notification history with synthetic IPC; no microphone or audio.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
  const dialog=()=>page.getByRole("dialog",{name:"通知一覧",exact:true});
  const live=()=>dialog().locator("[data-history-id]").filter({hasText:"録音状況の確認:"});
  await page.evaluate(()=>{const f=window.closeFixture;f.failJournal=true;return f.recordChanged({phase:"recording",sessionId:crypto.randomUUID()});});
  await page.locator('.toast-host .toast-card').filter({hasText:"5秒以上"}).waitFor();
  await page.getByRole("button",{name:"通知一覧",exact:true}).click();await dialog().waitFor();
  check((await live().innerText()).includes("自動で再確認"),"reason or next step missing");
  await page.evaluate(()=>window.closeFixture.failJournal=false);
  await page.waitForFunction(()=>document.querySelector('[data-history-id][data-resolution="resolved"]')?.textContent.includes("接続記録の保存を確認"));
  check(await live().getAttribute("data-resolution")==="resolved","fresh evidence did not resolve history");
  check(await live().locator("details:not([open])").count()===1,"old warning still appears as a current error");
  await page.locator(".toast-host .toast-card").filter({hasText:"録音状況の確認:"}).waitFor({state:"detached"});
  results.push("five-second reason is visible; verified recovery labels history and collapses the original warning");
  await page.evaluate(()=>window.closeFixture.journalStatus={state:"pending",requestedRevision:2,committedRevision:1,closed:false});
  await page.waitForFunction(()=>[...document.querySelectorAll('[data-history-id]')].some(e=>e.dataset.resolution==="unresolved"&&e.textContent.includes("録音状況の確認:")));
  const owner=await page.evaluate(()=>window.closeFixture.recording.sessionId);
  await page.evaluate(()=>window.closeFixture.recordChanged({phase:"saved"}));
  await page.waitForFunction(()=>[...document.querySelectorAll('[data-history-id]')].some(e=>e.dataset.resolution==="unconfirmed"&&e.textContent.includes("まだ再確認できていません")));
  await page.evaluate(owner=>{const f=window.closeFixture;const row=f.newArchivedRow({sessionId:owner,count:0,title:"保存確認の会議"});f.archived.rows=[row];return f.recordChanged({phase:"idle"});},owner);
  await page.waitForFunction(()=>[...document.querySelectorAll('[data-history-id]')].filter(e=>e.textContent.includes("録音状況の確認:")).every(e=>e.dataset.resolution==="resolved"));
  results.push("stopping leaves an unconfirmed result; final saved history later resolves the same recording");
  await page.evaluate(()=>{const f=window.closeFixture;const row=f.newArchivedRow({count:0,title:"通知読取りの会議"});row.unavailable=true;f.archived.rows.push(row);return f.recordChanged({phase:"idle"});});
  const archive=()=>dialog().locator('[data-history-id]').filter({hasText:"終了した録音「通知読取りの会議」"});
  await archive().waitFor();check((await archive().innerText()).includes("読み取れなかったため"),"archive read reason missing");
  check(!(await archive().innerText()).includes("欠損範囲"),"unreadable notification claims an audio gap");
  await page.evaluate(()=>{const f=window.closeFixture;f.archived.rows.at(-1).unavailable=false;return f.recordChanged({phase:"idle"});});
  await page.waitForFunction(()=>[...document.querySelectorAll('[data-history-id]')].find(e=>e.textContent.includes("終了した録音「通知読取りの会議」"))?.dataset.resolution==="resolved");
  results.push("archive read failures explain the cause without asserting audio loss and resolve on successful reread");
  for(const [width,theme]of [[960,"light"],[360,"dark"]]){
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme:theme});
    await live().first().locator("summary").click();
    check(await dialog().evaluate(e=>e.scrollWidth<=e.clientWidth+1),"expanded reason overflowed the notification list");
    await page.screenshot({path:`diagnostic-history-${width}-${theme}.png`});
    await live().first().locator("summary").click();
  }
  const logs=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="record_input_diagnostic_log"));
  check(logs.some(c=>c.args.stage==="live-journal"&&c.args.message?.includes("模擬履歴診断読込失敗")),"read cause was not logged");
  check(logs.some(c=>c.args.stage==="live-journal"&&c.args.message===null),"read recovery was not logged");
  results.push("light/dark wide/narrow history fits; local logs contain read failure detail and recovery");
  return {passed:results.length,results};
}
