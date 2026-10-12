// Held synthetic generation admission, no real model or recording.
async (page,capturePath)=>{
  const check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.goto("http://127.0.0.1:1420/?empty=1");
  await page.getByRole("button",{name:"議事録UI確認",exact:true}).click();
  await page.getByRole("tab",{name:"議事録",exact:true}).click();
  const button=page.getByRole("button",{name:"議事録を生成",exact:true});await button.waitFor();
  await page.evaluate(()=>{window.__mon.startMode="held";window.__mon.pref.payload.standard.modelPath="/synthetic/global.gguf";window.__mon.emitPlain("model-profile-changed");});
  const header=page.locator(".meeting-detail-status"),height=await header.evaluate(e=>e.getBoundingClientRect().height);
  await button.click();
  await page.waitForFunction(()=>document.querySelector('.minutes-card button[aria-label="議事録を生成"]')?.disabled || [...document.querySelectorAll('.minutes-card button')].some(b=>b.textContent==="議事録を生成" && b.disabled));
  check(await page.locator('.toast-host .toast-body p').filter({hasText:/操作を実行中|編集の保存を確認中/}).count()===0,"routine operation progress toast remains");
  check(!(await header.innerText()).includes("操作を実行中"),"busy text still appears in the title area");
  check(Math.abs(await header.evaluate(e=>e.getBoundingClientRect().height)-height)<1,"generation changed title height");
  check(await button.isDisabled(),"generation admission allowed duplicate submission");
  await page.screenshot({path:capturePath("minutes-progress-toast.png")});
  await page.evaluate(()=>window.__mon.startRelease());
  await page.waitForFunction(()=>!document.querySelector(".meeting-detail-status")?.innerText.includes("操作を実行中"));
  return {passed:1,results:["generation admission stays quiet without moving the header or enabling duplicate submission"]};
}
