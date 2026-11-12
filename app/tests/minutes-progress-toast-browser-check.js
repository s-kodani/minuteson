// Held synthetic generation admission, no real model or recording.
async (page,capturePath)=>{
  const check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.goto("http://127.0.0.1:1420/?empty=1");
  await page.getByRole("button",{name:"議事録UI確認",exact:true}).click();
  await page.getByRole("tab",{name:"議事録",exact:true}).click();
  const button=page.getByRole("button",{name:"議事録を生成",exact:true});await button.waitFor();
  await page.evaluate(()=>{window.__mon.startMode="held";window.__mon.pref.payload.standard.modelPath="/synthetic/global.gguf";window.__mon.emitPlain("model-profile-changed");});
  await page.evaluate(()=>document.fonts.ready);
  const header=page.locator(".meeting-detail-status"),title=header.locator(".meeting-title-row"),height=await title.evaluate(e=>e.getBoundingClientRect().height);
  await button.click();
  await page.waitForFunction(()=>document.querySelector('.minutes-card button[aria-label="議事録を生成"]')?.disabled || [...document.querySelectorAll('.minutes-card button')].some(b=>b.textContent==="議事録を生成" && b.disabled));
  check(await page.locator('.toast-host .toast-body p').filter({hasText:/操作を実行中|編集の保存を確認中/}).count()===0,"routine operation progress toast remains");
  check(!(await header.innerText()).includes("操作を実行中"),"busy text still appears in the title area");
  check(Math.abs(await title.evaluate(e=>e.getBoundingClientRect().height)-height)<1,"generation changed title row height");
  check(await header.locator(".operation-progress").count()===0,"operation progress is inside the title area");
  check(await button.isDisabled(),"generation admission allowed duplicate submission");
  const common=page.getByRole("region",{name:"進行中の処理",exact:true});
  await common.getByText("議事録生成 · 開始準備中",{exact:true}).waitFor();
  await common.locator('.progress-disclosure[aria-expanded="false"]').waitFor();
  const ring=common.locator('.progress-ring');
  check(await ring.getAttribute("data-spinning")==="true" && await ring.getAttribute("data-value")===null,"pending minutes preparation shows a dash instead of a spinner");
  check(await ring.locator(".progress-ring-arc").count()===1,"pending minutes preparation has no progress ring arc");
  check(await ring.locator("svg").evaluate(e=>getComputedStyle(e).animationName)==="progress-ring-spin","pending minutes preparation ring does not rotate");
  await page.screenshot({path:capturePath("minutes-progress-toast.png")});
  await page.evaluate(()=>window.__mon.startRelease());
  await page.waitForFunction(()=>!document.querySelector(".meeting-detail-status")?.innerText.includes("操作を実行中"));
  return {passed:2,results:["generation admission stays quiet without changing title typography or enabling duplicate submission","pending minutes preparation collapses after five seconds with a rotating ring and no dash"]};
}
