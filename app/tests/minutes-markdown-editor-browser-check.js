// Real editor/renderer, synthetic storage and interval callbacks; no audio.
async page=>{
  const check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.setViewportSize({width:1280,height:820});
  await page.goto("http://127.0.0.1:1420/tests/minutes-markdown-editor-browser.html");
  const edit=page.getByRole("button",{name:"議事録を編集",exact:true});
  await edit.waitFor();
  await page.getByRole("heading",{name:"点検会議",exact:true}).waitFor();
  check(await page.evaluate(()=>window.plainMinutesFixture.calls.length===0&&window.plainMinutesFixture.plays.length===0),"render saved or played implicitly");
  await edit.click();
  const input=page.getByRole("textbox",{name:"議事録のMarkdown本文",exact:true});
  await input.fill("# 人間の編集\n\n- **点検日**\n\n[区間](audio:00:00:01.000001-00:00:02.999999)\n");
  await page.getByRole("status").filter({hasText:"未保存"}).waitFor();
  check(await page.evaluate(()=>window.plainMinutesFixture.calls.length===0),"editing autosaved");
  const textareaIdentity=await input.evaluate(e=>{e.dataset.identity="same-native-textarea";return e.dataset.identity;});
  await page.getByRole("button",{name:"議事録のプレビューを表示",exact:true}).click();
  await page.getByRole("heading",{name:"人間の編集",exact:true}).waitFor();
  await page.getByRole("button",{name:"この区間の開始時刻から再生: 00:00:01.000001–00:00:02.999999",exact:true}).click();
  check(await page.evaluate(()=>JSON.stringify(window.plainMinutesFixture.plays)===JSON.stringify([{startUs:1000001,endUs:2999999}])),"interval precision or callback lost");
  await edit.click();
  check(await input.getAttribute("data-identity")===textareaIdentity,"preview replaced the native textarea");
  await page.evaluate(()=>window.plainMinutesFixture.mode="failure");
  await page.getByRole("button",{name:"保存",exact:true}).click();
  await page.getByRole("alert").waitFor();
  check((await input.inputValue()).includes("人間の編集"),"failed save lost input");
  await page.evaluate(()=>window.plainMinutesFixture.mode="success");
  await page.getByRole("button",{name:"保存を再試行",exact:true}).click();
  await page.getByRole("status").filter({hasText:"保存済み"}).waitFor();
  check(await page.evaluate(()=>{const c=window.plainMinutesFixture.calls;return c.length===2&&JSON.stringify(c[0])===JSON.stringify(c[1]);}),"retry changed save operation or body");
  await input.fill("クリック時の保存内容\n");
  await page.evaluate(()=>window.plainMinutesFixture.mode="held");
  await page.getByRole("button",{name:"保存",exact:true}).click();
  await page.getByRole("status").filter({hasText:"保存中"}).waitFor();
  await input.fill("保存待ち中に人間が追記\n");
  await page.evaluate(()=>{window.plainMinutesFixture.mode="success";window.plainMinutesFixture.release();});
  await page.getByRole("status").filter({hasText:"未保存"}).waitFor();
  check(await input.inputValue()==="保存待ち中に人間が追記\n","late response overwrote newer input");
  check(await page.evaluate(()=>window.plainMinutesFixture.calls.at(-1).markdown==="クリック時の保存内容\n"&&window.plainMinutesFixture.calls.length===3),"save drained later human input");
  await page.getByRole("button",{name:"保存",exact:true}).click();
  await page.getByRole("status").filter({hasText:"保存済み"}).waitFor();
  await input.fill("保存済みだが成功応答だけ失われた本文\n");
  await page.evaluate(()=>window.plainMinutesFixture.mode="lost");
  await page.getByRole("button",{name:"保存",exact:true}).click();
  await page.getByRole("alert").waitFor();
  await input.fill("成功応答待ちの後に追加した人間の本文\n");
  await page.evaluate(()=>window.plainMinutesFixture.mode="success");
  await page.getByRole("button",{name:"保存を再試行",exact:true}).click();
  await page.getByRole("status").filter({hasText:"未保存"}).waitFor();
  check(await input.inputValue()==="成功応答待ちの後に追加した人間の本文\n","lost success retry removed subsequent human input");
  check(await page.evaluate(()=>{const c=window.plainMinutesFixture.calls;return c.length===6&&JSON.stringify(c[4])===JSON.stringify(c[5]);}),"lost success retry did not use the identical saved request");
  await page.getByRole("button",{name:"保存",exact:true}).click();
  await page.getByRole("status").filter({hasText:"保存済み"}).waitFor();
  // Human deletion is intentional, not a generation-output validation error.
  await input.fill("");
  await input.press("Control+s");
  await page.getByRole("status").filter({hasText:"保存済み"}).waitFor();
  check(await page.evaluate(()=>window.plainMinutesFixture.calls.at(-1).markdown===""&&window.plainMinutesFixture.calls.length===8),"explicit empty human document was not saved");
  await input.fill("# 最終プレビュー\n\n| 項目 | 状態 |\n| --- | --- |\n| 点検日 | 未確認 |\n\n<script>alert(1)</script>\n\n![外部画像](https://example.com/private.png)\n");
  await page.getByRole("button",{name:"議事録のプレビューを表示",exact:true}).click();
  check(await page.locator(".minutes-markdown-preview script,.minutes-markdown-preview img").count()===0,"preview fetched images or executed HTML");
  for(const theme of ["light","dark"])for(const width of [1280,360]){
    await page.setViewportSize({width,height:720});await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),"plain editor overflows horizontally");
    await page.screenshot({path:`output/playwright/plain-minutes-editor-${theme}-${width}-20261012.png`});
  }
  return {passed:11,explicitSaving:true,nativeTextareaPreserved:true,lostSuccessInputRetained:true,emptyHumanDocumentSaved:true,realCapture:false,realAI:false};
}
