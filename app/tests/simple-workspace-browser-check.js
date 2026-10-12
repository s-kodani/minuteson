// Only synthetic IPC; no user meetings, audio or OS settings are modified.
async (page,capturePath)=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  const title=page.locator(".meeting-title-row h1"),edit=page.getByRole("button",{name:"会議名を変更",exact:true});
  await edit.waitFor();
  const gear=page.locator(".settings-nav-glyph svg");check(await gear.evaluate(e=>{const r=e.getBoundingClientRect();return r.width===28&&r.height===28;}),"settings gear is not 28px");
  results.push("settings gear has consistent 28px geometry");
  await edit.click();const dialog=page.getByRole("dialog",{name:"会議名を変更",exact:true}),input=dialog.getByRole("textbox",{name:"会議名",exact:true});
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
  return {passed:results.length,results};
}
