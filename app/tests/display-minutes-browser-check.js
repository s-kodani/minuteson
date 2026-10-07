// Production minutes reader/editor and fixed transcript, synthetic IPC only.
async (page,capturePath,detailUI) => {
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  await page.reload();await page.waitForFunction(()=>!!window.__mon);
  await page.getByRole("button",{name:"開く",exact:true}).first().click();await detailUI.openMinutes();
  const dock=page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true});
  await dock.getByText("再生トラック",{exact:true}).click();
  const size=dock.getByRole("combobox",{name:"本文の文字サイズ",exact:true});
  const expectFont=async value=>page.waitForFunction(value=>getComputedStyle(document.documentElement).getPropertyValue("--reading-font-size").trim()===`${value}px`,value);
  for(const value of [13,15,18,20]){
    await size.selectOption(String(value));await expectFont(value);
    check(await page.locator(".minutes-document").first().evaluate((e,value)=>parseFloat(getComputedStyle(e).fontSize)===value&&parseFloat(getComputedStyle(e.querySelector("h1")).fontSize)===value*1.6,value),"minutes body/headings do not scale together");
  }
  results.push("all four text sizes scale the production minutes reader and its relative headings");
  await page.getByRole("button",{name:"議事録を編集",exact:true}).click();
  const rich=page.getByRole("textbox",{name:"議事録ブロック1",exact:true});
  await page.evaluate(()=>{window.__mon.failMinutesSave=true;});await rich.fill("文字サイズで未保存の議事録を保持");
  await page.getByRole("status").filter({hasText:"議事録の保存要確認"}).waitFor();
  await rich.evaluate(e=>{window.displayRich=e;const r=document.createRange();r.setStart(e.firstChild,2);r.setEnd(e.firstChild,6);getSelection().removeAllRanges();getSelection().addRange(r);});
  const saves=await page.evaluate(()=>window.__mon.calls.filter(c=>c.cmd==="save_minutes_draft").length);
  for(const value of [13,20])for(const theme of ["light","dark"]){
    await size.selectOption(String(value));await expectFont(value);
    await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
    check(await rich.textContent()==="文字サイズで未保存の議事録を保持"&&await rich.evaluate(e=>{const s=getSelection();return e===window.displayRich&&s.rangeCount===1&&s.getRangeAt(0).startContainer===e.firstChild&&s.getRangeAt(0).startOffset===2&&s.getRangeAt(0).endOffset===6;}),"font/theme replaced rich content or range");
    check(await page.locator(".minutes-wysiwyg").evaluate((e,value)=>parseFloat(getComputedStyle(e).fontSize)===value,value)&&await page.getByRole("button",{name:"編集を終了",exact:true}).isVisible(),"font not applied to rich editor or editing ended");
  }
  check(await page.evaluate(()=>window.__mon.calls.filter(c=>c.cmd==="save_minutes_draft").length)===saves,"display change retried a failed document save");
  results.push("failed rich-text save retains editing, field identity, content and range across sizes/themes; display changes never retry document publication");
  await page.screenshot({path:"display-minutes-unsaved-dark-20.png"});
  await page.evaluate(()=>{window.__mon.failMinutesSave=false;});await page.getByRole("button",{name:"議事録保存を再試行",exact:true}).first().click();
  await page.waitForFunction(()=>window.__mon.drafts.a.payload.blocks[0].content[0].text==="文字サイズで未保存の議事録を保持");
  await page.getByRole("button",{name:"編集を終了",exact:true}).click();
  await page.locator(".evidence-chip").first().click();
  const evidence=page.getByRole("complementary",{name:"項目の固定根拠一覧",exact:true});await evidence.waitFor();
  await evidence.getByRole("button",{name:"固定文字起こし全文へ移動",exact:true}).first().click();
  const full=page.getByRole("region",{name:"固定文字起こし全文",exact:true});await full.waitFor();
  check(await full.locator(".document-text").first().evaluate(e=>getComputedStyle(e).fontSize==="20px"),"fixed full transcript ignored confirmed size");
  results.push("fixed generation-input transcript inherits the same confirmed reading size without changing frozen evidence");
  return {passed:results.length,results};
}
