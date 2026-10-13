// Only synthetic IPC; no user meetings, audio or OS settings are modified.
async (page,capturePath)=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  const title=page.locator(".meeting-title-row h1"),edit=page.getByRole("button",{name:"会議名を変更",exact:true});
  await edit.waitFor();
  const gear=page.locator(".settings-nav-glyph svg");check(await gear.evaluate(e=>{const r=e.getBoundingClientRect();return r.width===28&&r.height===28;}),"settings gear is not 28px");
  results.push("settings gear has consistent 28px geometry");
  await edit.click();const dialog=page.getByRole("dialog",{name:"会議名を変更",exact:true}),input=dialog.getByRole("textbox",{name:"会議名",exact:true});
  await input.waitFor();
  for(const theme of ["light","dark"]){
    await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;},theme);
    for(const width of [1200,390]){
      await page.setViewportSize({width,height:800});
      const metrics=await input.evaluate(e=>{const s=getComputedStyle(e),r=e.getBoundingClientRect(),d=e.closest('dialog').getBoundingClientRect(),p=e.closest('label').getBoundingClientRect(),save=e.closest('form').querySelector('button[type=submit]'),b=save.getBoundingClientRect();return {height:r.height,buttonHeight:b.height,border:s.borderTopStyle,borderWidth:s.borderTopWidth,radius:s.borderTopLeftRadius,font:s.fontSize,padding:s.paddingLeft,background:s.backgroundColor,surface:getComputedStyle(e.closest('dialog')).backgroundColor,width:r.width,labelWidth:p.width,left:r.left,right:r.right,dialogLeft:d.left,dialogRight:d.right};});
      check(metrics.height===40&&metrics.buttonHeight===40,"rename field and buttons must share 40px height");
      check(metrics.border==="solid"&&metrics.borderWidth==="1px"&&metrics.radius==="8px","rename field uses native inset/square borders");
      check(metrics.font==="13px"&&metrics.padding==="12px"&&metrics.background===metrics.surface,"rename field typography or surface differs");
      check(Math.abs(metrics.width-metrics.labelWidth)<1&&metrics.left>metrics.dialogLeft&&metrics.right<metrics.dialogRight,"rename field lacks dialog gutters or overflows");
      await page.screenshot({path:capturePath(`rename-input-${theme}-${width}.png`)});
    }
  }
  await page.setViewportSize({width:1200,height:800});
  results.push("rename field has consistent flat rounded styling, typography, height and gutters in both themes and widths");
  await input.fill("取り消す会議名");await dialog.getByRole("button",{name:"キャンセル",exact:true}).click();
  await dialog.waitFor({state:"hidden"});check(await title.innerText()==="会議A","cancel renamed meeting");
  await page.waitForFunction(()=>document.activeElement?.getAttribute("aria-label")==="会議名を変更");
  await edit.click();check(await input.inputValue()==="会議A","reopening retained cancelled title");
  await input.fill("");check(await dialog.getByRole("button",{name:"保存",exact:true}).isDisabled(),"blank title accepted");
  await input.fill("会議名を簡単に変更");await dialog.getByRole("button",{name:"保存",exact:true}).click();await dialog.waitFor({state:"hidden"});
  check(await title.innerText()==="会議名を簡単に変更","saved title missing");
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="rename_meeting").length)===1,"rename submitted more than once");
  results.push("title icon opens plain rename modal; cancel, blank guard, single versioned save and focus restoration work");
  await page.getByRole("button",{name:"会議管理・録音・解析操作",exact:true}).click();
  const tools=page.getByRole("dialog",{name:"会議管理・録音・解析操作",exact:true});await tools.waitFor();
  check(!(await tools.innerText()).includes("録音セッション")&&!(await tools.innerText()).includes("会議情報"),"removed management sections remain");
  await tools.locator(':scope > .common-modal-dismiss-row > [data-dialog-cancel]').click();await tools.waitFor({state:"hidden"});
  results.push("meeting operation menu contains no recording session or meeting info sections");
  for(const width of [1200,400]){await page.setViewportSize({width,height:800});const h=await title.boundingBox(),b=await edit.boundingBox();check(b.x>=h.x+h.width&&Math.abs((h.y+h.height/2)-(b.y+b.height/2))<2,"title edit icon lacks spacing or vertical alignment");await page.screenshot({path:capturePath(`simple-workspace-${width}.png`)});}
  await page.setViewportSize({width:1200,height:800});
  await page.getByRole("button",{name:"設定",exact:true}).click();
  await page.locator('.content h1').filter({hasText:/^設定$/}).waitFor();
  check(await page.getByRole("button",{name:"本文と再生の表示設定",exact:true}).count()===0,"removed display settings remain");
  check(await page.locator('.theme-settings select').count()===1,"settings should retain only the theme selector");
  check(await page.locator('.toast-host .toast-body p').filter({hasText:/操作を実行中|編集の保存を確認中/}).count()===0,"settings navigation shows a routine progress toast");
  results.push("settings keeps its theme selector without the removed reading panel or routine navigation notifications");
  await page.locator('.sidebar').getByRole("button",{name:"会議一覧",exact:true}).click();
  await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();
  check(await page.locator('.content .landing-tools').count()===0&&await page.getByRole("button",{name:"会議管理・録音・解析操作",exact:true}).count()===0&&await page.getByRole("button",{name:"その他の操作",exact:true}).count()===0,"meeting-list footer management trigger remains");
  check(await page.locator('.meeting-table').isVisible(),"removing footer hid meeting list");
  check(await page.getByRole("dialog",{name:"会議管理・録音・解析操作",exact:true}).count()===0,"meeting-list management dialog remains mounted");
  const row=page.locator('.meeting-table tbody tr').filter({hasText:"会議名を簡単に変更"});
  await row.getByRole("button",{name:"会議名を簡単に変更の操作",exact:true}).click();
  await page.getByRole("menuitem",{name:"開く",exact:true}).waitFor();
  check(await page.getByRole("menuitem",{name:"会議名を簡単に変更を削除",exact:true}).count()===1,"per-meeting deletion menu was removed with footer");
  await page.keyboard.press("Escape");
  await page.screenshot({path:capturePath("meeting-list-no-footer.png")});
  results.push("meeting list has no footer ellipsis or management dialog; per-meeting table remains available");
  return {passed:results.length,results};
}
