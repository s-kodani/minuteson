async(page)=>{
  const check=(value,message)=>{if(!value)throw Error(message);},results=[];
  await page.getByRole("navigation",{name:"メインナビゲーション"}).getByRole("button",{name:"設定",exact:true}).click();
  const trigger=page.getByRole("button",{name:"クレジット",exact:true});
  await trigger.scrollIntoViewIfNeeded();
  const footer=page.getByRole("group",{name:"アプリ情報",exact:true});
  check(JSON.stringify(await footer.getByRole("button").allTextContents())===JSON.stringify(["クレジット","免責事項","このアプリについて"]),"footer link order incorrect");
  check(await page.evaluate(()=>window.creditsFixture.about.calls.length===0),"app metadata loaded before opening");
  check(await page.evaluate(()=>window.creditsFixture.calls===0),"credits loaded before opening");
  await trigger.click();
  const modal=page.getByRole("dialog",{name:"クレジット",exact:true});
  await modal.getByRole("alert").waitFor();
  check((await modal.innerText()).includes("合成クレジット読込エラー"),"failure missing inside modal");
  await page.evaluate(()=>{window.creditsFixture.fail=false;});
  await modal.getByRole("button",{name:"再試行",exact:true}).click();
  await modal.getByRole("heading",{name:"主要な利用技術",exact:true}).waitFor();
  await modal.getByText("React",{exact:true}).waitFor();
  await modal.getByText("Qwen3-4B",{exact:true}).waitFor();
  check(await page.evaluate(()=>window.creditsFixture.calls===2),"retry count incorrect");
  results.push("settings link loads lazily; native errors and explicit retry stay inside credits modal");
  await modal.getByText("ライブラリ一覧（80件）",{exact:true}).click();
  await modal.getByText("dependency-79",{exact:true}).scrollIntoViewIfNeeded();
  await modal.getByText("dependency-79",{exact:true}).waitFor();
  const summary=modal.getByText("依存ライブラリの通知文",{exact:true});
  await summary.scrollIntoViewIfNeeded();await summary.click();
  const notice=modal.locator(".credits-notice pre");
  await notice.waitFor();
  await notice.evaluate(element=>{element.scrollTop=element.scrollHeight;});
  check((await notice.innerText()).includes("<script>window.creditMarkupExecuted=true</script>"),"notice not preserved exactly");
  check(await page.evaluate(()=>!window.creditMarkupExecuted),"notice executed markup");
  check(await notice.evaluate(element=>element.scrollTop>0),"long notice cannot scroll");
  results.push("versions, licenses, models, dependencies and long notices remain readable without interpreting markup");
  await page.keyboard.press("Escape");await modal.waitFor({state:"hidden"});
  check(await trigger.evaluate(element=>element===document.activeElement),"focus did not return to credits link");
  await trigger.click();await modal.waitFor();
  check(await page.evaluate(()=>window.creditsFixture.calls===2),"reopening reloaded static credits unnecessarily");
  await modal.getByRole("button",{name:"クレジットを閉じる",exact:true}).click();
  await modal.waitFor({state:"hidden"});
  results.push("Escape and close button dismiss; focus returns to settings link and reopening reuses loaded credits");

  const disclaimerTrigger=footer.getByRole("button",{name:"免責事項",exact:true});
  const disclaimer=page.getByRole("dialog",{name:"免責事項",exact:true});
  await disclaimerTrigger.click();await disclaimer.waitFor();
  check(JSON.stringify(await disclaimer.getByRole("heading",{level:3}).allTextContents())===JSON.stringify(["AIの出力","録音と権利への配慮","データの保全","無保証・責任の制限"]),"disclaimer sections missing");
  check((await disclaimer.innerText()).includes("ライセンス原文に従います"),"license priority missing");
  check(await page.evaluate(()=>window.creditsFixture.about.calls.length===0&&window.creditsFixture.calls===2),"disclaimer fetched unrelated resources");
  await page.keyboard.press("Tab");
  check(await disclaimer.evaluate(element=>element.contains(document.activeElement)),"Tab escaped disclaimer");
  await page.keyboard.press("Escape");await disclaimer.waitFor({state:"hidden"});
  check(await disclaimerTrigger.evaluate(element=>element===document.activeElement),"disclaimer did not restore focus");
  results.push("disclaimer contains four sections, needs no native reads, traps focus and restores its own opener");

  const aboutTrigger=footer.getByRole("button",{name:"このアプリについて",exact:true});
  const about=page.getByRole("dialog",{name:"このアプリについて",exact:true});
  await page.evaluate(()=>{window.creditsFixture.about.hold=true;});
  await aboutTrigger.click();await about.getByRole("status").waitFor();
  await page.waitForFunction(()=>window.creditsFixture.about.pending.length===2);
  await page.keyboard.press("Escape");await about.waitFor({state:"hidden"});
  check(await aboutTrigger.evaluate(element=>element===document.activeElement),"loading about did not restore focus");
  await aboutTrigger.click();await about.getByRole("status").waitFor();
  check(await page.evaluate(()=>window.creditsFixture.about.calls.length===2),"reopening duplicated pending metadata requests");
  await page.evaluate(()=>{const about=window.creditsFixture.about;about.hold=false;about.pending.splice(0).forEach(resolve=>resolve());});
  await about.getByRole("alert").waitFor();
  check((await about.innerText()).includes("合成アプリ情報読込エラー"),"metadata failure missing inside about");
  await about.getByText("MIT License",{exact:true}).waitFor();
  await page.evaluate(()=>{window.creditsFixture.about.fail=false;});
  await about.getByRole("button",{name:"再試行",exact:true}).click();
  await about.getByText("MinutesOn QA",{exact:true}).waitFor();
  await about.getByText("9.8.7-preview.2",{exact:true}).waitFor();
  await about.getByText("s-kodani",{exact:true}).waitFor();
  await about.getByText("Copyright (c) 2026 s-kodani",{exact:true}).waitFor();
  check(await page.evaluate(()=>window.creditsFixture.about.calls.length===4),"metadata retry count incorrect");
  await about.getByRole("button",{name:"このアプリについてを閉じる",exact:true}).click();await about.waitFor({state:"hidden"});
  check(await aboutTrigger.evaluate(element=>element===document.activeElement),"about close button did not restore focus");
  await aboutTrigger.click();await about.getByText("9.8.7-preview.2",{exact:true}).waitFor();
  check(await page.evaluate(()=>window.creditsFixture.about.calls.length===4),"reopening did not reuse app metadata");
  await page.mouse.click(4,4);await about.waitFor({state:"hidden"});
  check(await aboutTrigger.evaluate(element=>element===document.activeElement),"backdrop dismissal did not restore focus");
  results.push("about metadata loads lazily, stays dismissible while pending, retries errors, reflects QA name and runtime version, and caches successful reads");

  for(const theme of ["light","dark"]){
    await page.emulateMedia({colorScheme:theme});
    await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
    for(const size of [{width:1280,height:820},{width:960,height:640},{width:390,height:600}]){
      await page.setViewportSize(size);await aboutTrigger.scrollIntoViewIfNeeded();
      check(await footer.evaluate(element=>{
        const rect=element.getBoundingClientRect(),buttons=[...element.querySelectorAll("button")].map(button=>button.getBoundingClientRect());
        return buttons.every(button=>button.left>=rect.left&&button.right<=rect.right+1&&button.bottom<=innerHeight)&&document.documentElement.scrollWidth<=innerWidth;
      }),`footer overflow at ${theme}/${size.width}`);
      if(size.width>=960)check(await footer.evaluate(element=>new Set([...element.querySelectorAll("button")].map(button=>Math.round(button.getBoundingClientRect().top))).size===1),"desktop footer links are not in one row");
      const capture=theme==="light"&&size.width===1280||theme==="dark"&&size.width===960;
      if(capture){await aboutTrigger.evaluate(element=>element.blur());await page.screenshot({path:`output/playwright/settings-footer-${theme}.png`});}
      for(const [opener,dialog,label] of [[disclaimerTrigger,disclaimer,"免責事項"],[aboutTrigger,about,"このアプリについて"]]){
        await opener.click();await dialog.waitFor();
        check(await dialog.evaluate(element=>{const rect=element.getBoundingClientRect();return rect.left>=0&&rect.right<=innerWidth&&rect.top>=0&&rect.bottom<=innerHeight&&element.scrollWidth<=element.clientWidth+1;}),`${label} overflow at ${theme}/${size.width}`);
        await dialog.getByRole("button",{name:`${label}を閉じる`,exact:true}).waitFor();
        if(capture)await page.screenshot({path:`output/playwright/settings-${label==="免責事項"?"disclaimer":"about"}-${theme}.png`});
        await dialog.evaluate(element=>{element.scrollTop=element.scrollHeight;});
        await dialog.locator(".common-modal-body p").last().scrollIntoViewIfNeeded();
        check(await dialog.locator(".common-modal-body p").last().isVisible(),`${label} bottom content inaccessible`);
        await page.keyboard.press("Escape");await dialog.waitFor({state:"hidden"});
      }
    }
  }
  results.push("footer and both modals fit light/dark themes at 1280, 960 and 390px, with scrolling and dismiss controls accessible");
  return results;
}
