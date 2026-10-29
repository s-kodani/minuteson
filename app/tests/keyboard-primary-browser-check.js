// Actual visible controls; synthetic meeting only, no device or OS settings.
async (page, _capturePath, detailUI) => {
  const primary = detailUI?.primaryModifier ?? "Meta";
  const check=(ok,message)=>{if(!ok)throw Error(message);};
  const button=name=>page.getByRole("button",{name,exact:true});
  const body=button("00:00:00の発言を編集");
  const field=page.getByRole("textbox",{name:"00:00:00の発言",exact:true});
  await button("会議A").press("Space");
  await body.press(`${primary}+f`);
  check(await page.getByRole("searchbox",{name:"発言を検索",exact:true}).evaluate(e=>e===document.activeElement),"primary+F lost transcript search");
  await body.press("Enter"); await field.waitFor();
  await page.waitForFunction(()=>document.activeElement?.getAttribute("aria-label")==="00:00:00の発言");
  await field.fill("キーボードでの合成訂正です。"); await field.press(`${primary}+Enter`);
  await body.waitFor(); check(await body.innerText()==="キーボードでの合成訂正です。","keyboard commit did not preserve text");
  check(await body.evaluate(e=>e===document.activeElement),"commit lost editable body focus");
  await body.press("Space"); await field.waitFor();
  await field.fill("取り消す合成入力です。"); await field.press("Escape");
  await body.waitFor(); check(await body.innerText()==="キーボードでの合成訂正です。","Escape kept cancelled text");
  check(await body.evaluate(e=>e===document.activeElement),"cancel lost editable body focus");
  await button("00:00:00の話者を変更").press("Space");
  const close=button("確認を閉じる"); await close.waitFor();
  await close.press("Shift+Tab"); await page.keyboard.press("Tab");
  check(await close.evaluate(e=>e===document.activeElement),"modal Tab did not wrap to safe close");
  await close.press("Escape");
  check(await button("00:00:00の話者を変更").evaluate(e=>e===document.activeElement),"modal dismissal lost keyboard entry");
  return {passed:1,results:["search, body Enter/Space, commit/cancel, individual speaker modal Tab and Escape through visible controls"]};
}
