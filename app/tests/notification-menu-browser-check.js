// Production UI, synthetic events only. Never opens native capture or models.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  await page.getByRole("heading",{name:"発言一覧",exact:true}).waitFor();
  const edit=page.getByRole("button",{name:"00:00:00の発言を編集",exact:true});
  await edit.click();const input=page.getByRole("textbox",{name:"00:00:00の発言",exact:true});await input.fill("通知を開いても未保存の入力を保持");
  await page.evaluate(()=>{for(let i=0;i<6;i++)window.monBrowserFixture.showToast({key:`menu-test-${i}`,message:`一覧だけの通知 ${i}`,level:i===0?"error":"success",eventId:`op-${i}`});});
  check(await page.locator(".issue-status,.archived-input-notices,.job-issues").count()===0,"inline notification history escaped into workspace");
  results.push("Sonner remains available while workspace history banners are removed");
  await page.getByRole("button",{name:"通知一覧",exact:true}).click();
  const dialog=page.getByRole("dialog",{name:"通知一覧",exact:true});await dialog.waitFor();
  check(await dialog.locator(".toast-card").filter({hasText:"一覧だけの通知"}).count()===6,"notification batch was truncated");
  check(await page.evaluate(()=>window.monBrowserFixture.draft===null),"opening menu saved unfinished transcription");
  results.push("all simultaneous notifications are listed without saving or replacing the current editor");
  await page.waitForTimeout(4300);check(await dialog.locator(".toast-card").filter({hasText:"一覧だけの通知"}).count()===6,"notifications auto-expired in menu");
  results.push("ordinary notifications remain until explicitly closed");
  await dialog.locator(".toast-card").filter({hasText:"一覧だけの通知 0"}).getByRole("button",{name:"エラー通知を閉じる",exact:true}).click();
  await page.evaluate(()=>window.monBrowserFixture.showToast({key:"menu-test-0",message:"一覧だけの通知 0",level:"error",eventId:"op-0"}));
  check(await dialog.locator(".toast-card").filter({hasText:"一覧だけの通知"}).count()===6,"dismissing toast deleted history");
  check(await page.evaluate(()=>!window.monBrowserFixture.notifications().some(n=>n.key==="menu-test-0")),"dismissed event replayed as a toast");
  await dialog.getByRole("button",{name:"通知一覧を閉じる",exact:true}).click();await dialog.waitFor({state:"hidden"});
  await page.waitForFunction(()=>document.activeElement?.getAttribute("aria-label")==="通知一覧");
  check(await input.inputValue()==="通知を開いても未保存の入力を保持","notification dismissal lost input");
  await page.getByRole("button",{name:"通知一覧",exact:true}).click();await dialog.waitFor();
  check(await dialog.locator(".toast-card").filter({hasText:"一覧だけの通知"}).count()===6,"reopening duplicated or lost notifications");
  results.push("dismissing a toast retains its history without replay; reopening preserves input and focus");
  for(const [width,theme]of [[1200,"light"],[400,"dark"]]){
    await page.setViewportSize({width,height:800});await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;},theme);
    const bounds=await dialog.boundingBox();check(bounds.x>=0&&bounds.x+bounds.width<=width&&bounds.y>=0&&bounds.y+bounds.height<=800,"menu outside viewport");
    check(await dialog.evaluate(e=>e.scrollWidth<=e.clientWidth+1),"horizontal notification scroll");
    check(await dialog.locator(".toast-body p").first().evaluate(e=>getComputedStyle(e).display==="block"&&getComputedStyle(e).overflow==="visible"),"notification text clamped");
    await page.screenshot({path:`notification-menu-${width}-${theme}.png`});
  }
  results.push("wide/light and narrow/dark notification menus wrap complete text without horizontal scrolling");
  await page.keyboard.press("Escape");await dialog.waitFor({state:"hidden"});
  check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(c=>["start_recording","start_capture_test","start_model_download"].includes(c.command))),"notification browsing started capture/download");
  results.push("Escape closes only the list; notification browsing does not start capture or downloads");
  return {passed:results.length,results};
}
