async (page) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const entry = name => page.getByRole("button", { name, exact: true });
  const dialog = name => page.getByRole("dialog", { name, exact: true });
  const boundary = await entry("親の設定").getAttribute("aria-controls");
  check(boundary === await entry("別の設定").getAttribute("aria-controls"), "siblings use different native owners");
  for (let i=0;i<2;i++) {
    await entry("親の設定").press("Space"); await dialog("親の設定").waitFor();
    await page.getByRole("textbox", { name:"親の入力",exact:true }).fill("保持する親入力");
    await entry("子の設定").press("Space"); await dialog("子の設定").waitFor();
    check(await page.locator("dialog[open]").count()===1,"nested settings created another native barrier");
    await page.getByRole("textbox", { name:"子の入力",exact:true }).fill("保持する子入力");
    await entry("変更確認を開く").press("Space"); await dialog("子設定の変更確認").waitFor();
    await entry("確認をキャンセル").press("Escape"); await dialog("子の設定").waitFor();
    check(await entry("変更確認を開く").evaluate(e=>e===document.activeElement),"confirmation lost child entry");
    check(await page.getByRole("textbox",{name:"子の入力",exact:true}).evaluate(e=>e.value)==="保持する子入力","child fields remounted");
    await entry("確認を閉じる").press("Escape"); await dialog("親の設定").waitFor();
    check(await entry("子の設定").evaluate(e=>e===document.activeElement),"child dismissal lost parent entry");
    check(await page.getByRole("textbox",{name:"親の入力",exact:true}).evaluate(e=>e.value)==="保持する親入力","parent fields remounted");
    await entry("確認を閉じる").press("Escape");
    check(await entry("親の設定").evaluate(e=>e===document.activeElement),"parent dismissal lost workspace entry");
    await entry("別の設定").press("Space"); await dialog("別の設定").waitFor();
    await entry("確認を閉じる").press("Escape");
  }
  for (const keepFocus of [false,true]) {
    await entry("読込設定").press("Space");
    await entry("合成読込を開く").press("Space");
    await dialog("読込中の確認画面").waitFor();
    check(!await entry("確認を閉じる").isEnabled(),"busy review dismissal became enabled");
    const finish = keepFocus ? "合成の読込を完了（焦点保持）" : "合成の読込を完了";
    await entry(finish).press("Space");
    await entry("確認を閉じる").waitFor();
    check(await entry(keepFocus ? finish : "確認を閉じる").evaluate(e=>e===document.activeElement),"review readiness lost or stole focus");
    await entry("確認を閉じる").press("Escape"); await dialog("読込設定").waitFor();
    check(await entry("合成読込を開く").evaluate(e=>e===document.activeElement),"ready review lost its opener");
    await entry("確認を閉じる").press("Escape");
  }
  await page.getByText("実行回数 0",{exact:true}).waitFor();
  return {passed:1,results:["shared sibling/nested ownership, retained inputs, confirmation cancellation and focus return twice"]};
}
