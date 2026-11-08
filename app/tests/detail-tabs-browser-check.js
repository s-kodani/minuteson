// Production panels, synthetic IPC only; no native recording or audio.
async (page, capturePath, detailUI) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const tabs = page.getByRole("tablist", {name:"会議詳細",exact:true});
  const transcript = tabs.getByRole("tab", {name:"文字起こし",exact:true}), minutes = tabs.getByRole("tab", {name:"議事録",exact:true});
  const editor = page.getByRole("region", {name:"文字起こし編集",exact:true});
  const body = editor.getByRole("region", {name:"発言本文",exact:true});
  await page.evaluate(() => window.monBrowserFixture.setUtterances(Array.from({length:103}, (_,i) => ({id:"speech-"+i,trackId:"microphone",startUs:i*1_000_000,endUs:(i+1)*1_000_000,recognizedText:"本文"+i,effectiveText:"本文"+i,attributionState:"assigned",speakerIds:["speaker-a"]}))));
  await page.getByRole("button", {name:"会議A",exact:true}).click(); await body.waitFor();
  check(await transcript.getAttribute("aria-selected")==="true" && await page.getByRole("tabpanel").count()===1, "invalid initial tab");
  const holdNavigation = () => page.evaluate(async () => {
    const {registerPendingEdits} = await import("/src/draft-save.ts");
    let release;
    const wait = new Promise(resolve => { release = resolve; });
    const unregister = registerPendingEdits(() => wait, () => false);
    window.releaseNavigationCheck = () => { unregister(); release(); };
  });
  const sidebar = page.getByRole("navigation", {name:"メインナビゲーション"});
  const settings = sidebar.getByRole("button", {name:"設定",exact:true});
  const logs = sidebar.getByRole("button", {name:"内部ログ",exact:true});
  const checkArrow = locator => locator.evaluate(button => button.disabled && getComputedStyle(button).cursor === "default");
  await holdNavigation();
  await minutes.click();
  check(await checkArrow(minutes) && await checkArrow(settings), "clean tab switch shows a forbidden cursor");
  check(await body.evaluate(list => !!list.closest("[inert]") && list.querySelector(".utterance-stamp").matches(":disabled")), "navigation did not disable row interactions");
  const blocked = await body.evaluate(list => {
    const before = window.monBrowserFixture.calls.length;
    list.querySelector(".transcript-body-edit").click();
    list.querySelector(".utterance-stamp").click();
    list.querySelector(".options-modal-trigger").click();
    return {before, after:window.monBrowserFixture.calls.length};
  });
  check(blocked.before === blocked.after && await body.locator("textarea").count() === 0
    && await page.getByRole("dialog").count() === 0, "memoized row handlers bypassed the navigation lock");
  results.push("native and programmatic row activation respect the current navigation lock");
  await settings.evaluate(button => button.click());
  await page.evaluate(() => window.releaseNavigationCheck());
  await page.waitForFunction(() => document.getElementById("meeting-tab-minutes")?.getAttribute("aria-selected") === "true");
  await transcript.click(); await body.waitFor();
  results.push("clean tab wait keeps the arrow cursor and blocks further clicks");
  await holdNavigation();
  await settings.click();
  check(await checkArrow(settings) && await checkArrow(logs), "clean screen switch shows a forbidden cursor");
  await logs.evaluate(button => button.click());
  await page.evaluate(() => window.releaseNavigationCheck());
  await page.getByRole("heading", {name:"設定",exact:true}).waitFor();
  await page.getByRole("button", {name:"前の画面へ戻る",exact:true}).click(); await body.waitFor();
  results.push("clean sidebar wait keeps the arrow cursor and blocks switching to another screen");
  await page.evaluate(() => { window.detailNodes=[document.querySelector(".transcript-card"),document.querySelector(".minutes-card")]; });
  await transcript.focus(); await transcript.press("ArrowRight");
  check(await minutes.evaluate(e=>e===document.activeElement) && await transcript.getAttribute("aria-selected")==="true", "arrow activated a tab without confirmation");
  await minutes.press("Enter"); await page.waitForFunction(()=>document.getElementById("meeting-tab-minutes")?.getAttribute("aria-selected")==="true");
  await minutes.press("Home"); await transcript.press("Space"); await body.waitFor();
  check(await page.evaluate(()=>window.detailNodes[0]===document.querySelector(".transcript-card") && window.detailNodes[1]===document.querySelector(".minutes-card")), "tab recreated panels");
  results.push("manual keyboard tab activation retains panel identity");
  for(const theme of ["light","dark"]) for(const width of [1280,1200,960]) {
    await page.setViewportSize({width,height:640}); await detailUI.applyPlaybackTheme(theme);
    for(const end of [false,true]) {
      await body.evaluate((e,end)=>{e.scrollTop=end?e.scrollHeight:0;},end);
      const boxes=await page.evaluate(()=>{
        const rect=s=>{const r=document.querySelector(s).getBoundingClientRect();return {top:r.top,bottom:r.bottom};};
        return {header:rect(".topbar"),tabs:rect(".meeting-detail-tabs"),main:rect(".content"),body:rect(".utterance-list"),dock:rect(".meeting-playback-dock"),height:innerHeight};
      });
      check(boxes.header.bottom<=boxes.tabs.top+1 && boxes.tabs.bottom<=boxes.main.top+1 && boxes.body.bottom<=boxes.dock.top+1 && boxes.dock.bottom<=boxes.height+1, "scrolling overlapped fixed chrome");
    }
  }
  results.push("two themes and three widths retain one body scrollport below fixed tabs and above playback");
  await body.evaluate(e=>{e.scrollTop=300;}); const position=await body.evaluate(e=>e.scrollTop);
  await minutes.click(); await transcript.click(); check(await body.evaluate(e=>e.scrollTop)===position,"tab return lost reading position");
  results.push("tab return keeps reading position");
  await page.setViewportSize({width:1200,height:900});
  const row=editor.getByRole("article",{name:"00:00:00の発言行",exact:true});
  const open=()=>row.getByRole("button",{name:"00:00:00の発言を編集",exact:true});
  const input=()=>row.getByRole("textbox",{name:"00:00:00の発言",exact:true});
  await open().click(); await input().fill("失敗時も保持する本文");
  await input().evaluate(e=>{window.detailInput=e;});
  await page.evaluate(()=>{window.monBrowserFixture.failSave=true;}); await minutes.click();
  await editor.getByRole("button",{name:"保存を再試行",exact:true}).waitFor();
  check(await transcript.getAttribute("aria-selected")==="true" && await input().evaluate(e=>e===window.detailInput) && await input().inputValue()==="失敗時も保持する本文", "failed tab save discarded input");
  results.push("failed save retains current tab and original input node");
  await page.evaluate(()=>{window.monBrowserFixture.failSave=false;}); await editor.getByRole("button",{name:"保存を再試行",exact:true}).click();
  await input().waitFor({state:"hidden"}); await minutes.click(); await transcript.click();
  check(await open().innerText()==="失敗時も保持する本文","saved text missing after return");
  results.push("explicit retry saves and returns to the plain utterance preview");
  await open().click(); await input().fill("保存の確定後にタブを変更");
  await page.evaluate(()=>{window.monBrowserFixture.persistenceDelayMs=800;}); await minutes.click();
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==="save_transcript_draft" && c.args.payload.utterances.some(u=>u.effective_text==="保存の確定後にタブを変更")));
  check(await transcript.getAttribute("aria-selected")==="true" && await minutes.evaluate(button => button.disabled && getComputedStyle(button).cursor === "not-allowed"),"tab did not retain its save lock before the receipt");
  await page.waitForFunction(()=>document.getElementById("meeting-tab-minutes")?.getAttribute("aria-selected")==="true");
  await page.evaluate(()=>{window.monBrowserFixture.persistenceDelayMs=0;}); await transcript.click();
  check(await open().innerText()==="保存の確定後にタブを変更","save omitted on tab switch");
  results.push("delayed save keeps the original tab until acknowledgement");
  await page.screenshot({path:"detail-tabs-simple.png"});
  return {passed:results.length,results};
}
