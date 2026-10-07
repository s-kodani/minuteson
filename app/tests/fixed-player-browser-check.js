// Production layout/player; IPC returns synthetic silence, not native recording.
async (page, capturePath, detailUI) => {
  const results = [],check = (v,m) => { if(!v) throw Error(m); };
  const editor = page.getByRole("region",{ name: "文字起こし編集",exact: true });
  const dock = page.getByRole("region",{ name: "会議の共通音声プレーヤー",exact: true });
  const player = dock.getByRole("group",{ name: "発言の音声再生",exact: true });
  const workspace = editor.getByRole("region",{ name: "発言本文",exact: true });
  const nav = page.getByRole("navigation",{ name: "メインナビゲーション",exact: true });
  const row = editor.getByRole("article",{ name: "00:00:00の発言行",exact: true });
  const counts = () => page.evaluate(() => { const calls = window.monBrowserFixture.calls; return { prepare: calls.filter(c => c.command === "prepare_utterance_playback").length,close: calls.filter(c => c.command === "close_playback").length,body: calls.filter(c => c.command === "save_transcript_draft").length }; });
  const open = async () => {
    await page.waitForFunction(() => !!window.monBrowserFixture && !document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
    if (await page.locator(".meeting-workspace-tools").count()) await detailUI.openMeetingTools();
    await page.getByRole("button",{ name: "開く",exact: true }).first().click();
    await editor.getByRole("button",{ name: "編集を開始",exact: true }).waitFor();
  };
  const boot = async () => { await page.reload(); await open(); };
  const play = async () => { await row.getByRole("button",{ name: "音声を再生",exact: true }).click(); await player.getByRole("button",{ name: "一時停止",exact: true }).waitFor(); };
  const pause = async () => { await player.getByRole("button",{ name: "一時停止",exact: true }).click(); await player.getByRole("button",{ name: "再生",exact: true }).waitFor(); };
  const bounds = () => page.evaluate(() => {
    const m = document.querySelector(".content"),d = document.querySelector(".meeting-playback-dock"),h = document.querySelector(".topbar"),t = document.querySelector(".meeting-detail-tabs");
    const b = el => { const r = el.getBoundingClientRect(); return { top:r.top,bottom:r.bottom,left:r.left,right:r.right }; };
    return { main:b(m),dock:b(d),header:b(h),tabs:b(t),height:innerHeight,width:innerWidth,pageHeight:document.documentElement.scrollHeight,pageWidth:document.documentElement.scrollWidth,pageScroll:scrollY,dockOverflow:d.scrollWidth > d.clientWidth + 1 };
  });

  await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
  check(await dock.count() === 0,"unselected meeting exposes active dock"); await open();
  check(await dock.isVisible() && (await dock.innerText()).includes("会議A") && (await counts()).prepare === 0 && await player.count() === 0,"idle dock prepared audio or omitted owner");
  results.push("unselected meeting hides dock; selected idle dock shows its owner without preparing or auto-playing audio");
  await play();
  check(await row.getAttribute("aria-current") === "true" && await row.getByText("再生中",{ exact:true }).isVisible(),"playing row lacks textual/current indicator");
  await pause(); check(await row.getAttribute("aria-current") === null && await row.getByText("再生中",{ exact:true }).count() === 0,"paused row still claims playing");
  check(await player.evaluate(e => !e.closest(".content") && !e.closest(".transcript-card")) && (await dock.innerText()).includes("同期精度は未検証"),"player still belongs to scrollport or hid alignment warning");
  results.push("selected utterance controls and alignment warning render outside the scrolling body");
  await player.evaluate(e => { window.monBrowserFixture.playerNode = e; });
  const prepareBefore = (await counts()).prepare;
  for(const width of [1280,1200,960]) {
    await page.setViewportSize({ width,height:640 });
    for(const end of [false,true]) {
      await workspace.evaluate((e,last) => { e.scrollTop = last ? e.scrollHeight : 0; },end);
      const b = await bounds();
      check(b.header.top === 0 && b.tabs.top === b.header.bottom && b.main.top === b.tabs.bottom && Math.abs(b.main.bottom - b.dock.top) <= 1 && Math.abs(b.dock.bottom - b.height) <= 1 && b.pageHeight <= b.height + 1 && b.pageWidth <= width && !b.dockOverflow && b.pageScroll === 0,`body/dock overlap or overflow ${JSON.stringify(b)}`);
      check(await player.evaluate(e => e === window.monBrowserFixture.playerNode),"scroll/resize remounted player");
    }
    await player.getByRole("button",{ name:"再生",exact:true }).focus();
    check(await player.getByRole("button",{ name:"再生",exact:true }).evaluate(e => e.getBoundingClientRect().bottom <= innerHeight),"focused primary control hidden");
  }
  check((await counts()).prepare === prepareBefore,"scroll/resize acquired another token");
  results.push("1280/1200/960 by 640 reserve dock space at both scroll ends without overlay, remount or another token");
  await player.getByRole("combobox",{ name:"速度",exact:true }).selectOption("1.5");
  const slider = player.getByRole("slider",{ name:"発言内の再生位置",exact:true }); await slider.fill("144000");
  for(const theme of ["light","dark"]) {
    await page.getByRole("banner").getByRole("combobox",{ name:"表示テーマ",exact:true }).selectOption(theme);
    await page.waitForFunction(t => document.documentElement.dataset.theme === t,theme);
    check(await slider.inputValue() === "144000" && await player.getByRole("combobox",{ name:"速度",exact:true }).inputValue() === "1.5" && (await counts()).prepare === prepareBefore,"theme changed playback position/rate");
    await workspace.evaluate(e => { e.scrollTop = e.scrollHeight; }); await page.screenshot({ path:`fixed-player-${theme}-960.png` });
  }
  results.push("both themes retain paused position, speed and token while the fixed controls remain visible");
  await page.setViewportSize({ width:360,height:800 });
  check(!(await bounds()).dockOverflow && await player.getByRole("button",{ name:"再生",exact:true }).isVisible(),"narrow dock overflows or hides playback");
  await page.screenshot({ path:"fixed-player-dark-360.png" }); await page.setViewportSize({ width:960,height:640 });
  results.push("360px reflows player controls without horizontal overflow");

  await editor.getByRole("button",{ name:"編集を開始",exact:true }).click();
  await row.getByRole("button",{ name:"00:00:00の発言を編集",exact:true }).click();
  const text = row.getByRole("textbox",{ name:"00:00:00の発言",exact:true }); await text.fill("再生位置を変えても未確定本文を保持します。");
  // Starting a draft deliberately releases the previous analysis-only player.
  await play(); await pause();
  await text.evaluate(e => { window.monBrowserFixture.textNode = e; e.setSelectionRange(3,8); });
  await slider.fill("192000"); await player.getByRole("combobox",{ name:"速度",exact:true }).selectOption("2");
  check(await text.inputValue() === "再生位置を変えても未確定本文を保持します。" && await text.evaluate(e => e === window.monBrowserFixture.textNode && e.selectionStart === 3 && e.selectionEnd === 8) && (await counts()).body === 0,"dock actions confirmed or replaced editing text");
  results.push("seek and speed retain unconfirmed text, its DOM node and selection without body publication");
  await row.getByRole("button",{ name:"キャンセル",exact:true }).click(); await text.waitFor({ state:"hidden" });
  await row.getByRole("button",{ name:"発言を手動分割",exact:true }).click();
  const split = page.getByRole("dialog",{ name:"発言の手動分割",exact:true }); await split.waitFor();
  check(await split.getByRole("group",{ name:"発言の音声再生",exact:true }).count() === 1 && await dock.getByRole("group",{ name:"発言の音声再生",exact:true }).count() === 0,"split has duplicate/inert-only audio controls");
  await page.keyboard.press("Escape"); await split.waitFor({ state:"hidden" }); check(await player.count() === 0 && (await dock.innerText()).includes("発言の時刻"),"split close revived detached player");
  results.push("split dialog owns its audio controls exclusively; close returns fixed dock to idle without reviving playback");

  await boot(); await page.evaluate(() => { window.monBrowserFixture.failPlayback = true; }); await row.getByRole("button",{ name:"音声を再生",exact:true }).click();
  await dock.getByRole("alert").waitFor(); await workspace.evaluate(e => { e.scrollTop = e.scrollHeight; });
  check((await dock.innerText()).includes("ハッシュ") && await dock.getByRole("alert").isVisible(),"playback failure lost below content");
  results.push("failed saved-audio verification stays visible in dock instead of being reported as successful playback");
  await boot(); await play(); await nav.getByRole("button",{ name:"設定",exact:true }).click();
  await dock.waitFor({ state:"hidden" }); await page.waitForFunction(() => window.monBrowserFixture.calls.some(c => c.command === "close_playback"));
  await nav.getByRole("button",{ name:"概要",exact:true }).click(); await dock.waitFor(); check(await player.count() === 0,"settings return resurrected old audio");
  results.push("settings navigation hides dock and releases old token; returning does not auto-play");
  await play(); const beforeSwitch = (await counts()).close; await detailUI.openMeetingTools(); await page.getByRole("button",{ name:"開く",exact:true }).nth(1).click();
  await page.waitForFunction(() => document.querySelector(".meeting-playback-content")?.getAttribute("data-meeting-id") === "meeting-1");
  await page.waitForFunction(n => window.monBrowserFixture.calls.filter(c => c.command === "close_playback").length > n,beforeSwitch);
  check((await dock.innerText()).includes("会議B") && await player.count() === 0,"different meeting adopted old audio");
  results.push("meeting switch changes dock owner and releases old playback without auto-playing another meeting");

  await boot(); await editor.getByRole("button",{ name:"編集を開始",exact:true }).click(); await row.getByRole("button",{ name:"00:00:00の発言を編集",exact:true }).click(); await play(); await pause();
  await text.fill("保存に失敗しても会議と音声操作を保持します。"); await page.evaluate(() => { window.monBrowserFixture.failSave = true; });
  await nav.getByRole("button",{ name:"設定",exact:true }).click(); await page.getByRole("alert").filter({ hasText:"編集内容はこの画面に保持" }).waitFor();
  check(await dock.isVisible() && await player.count() === 1 && await text.inputValue() === "保存に失敗しても会議と音声操作を保持します。" && (await dock.innerText()).includes("会議A"),"failed save hid dock or lost owner/input");
  results.push("failed navigation save preserves meeting owner, fixed player and unconfirmed text");

  await boot(); await page.evaluate(() => { window.monBrowserFixture.holdPlayback = true; }); await row.getByRole("button",{ name:"音声を再生",exact:true }).click();
  await page.waitForFunction(() => typeof window.monBrowserFixture.releasePlayback === "function");
  check((await dock.innerText()).includes("再生を準備"),"pending audio lacks fixed progress");
  await nav.getByRole("button",{ name:"設定",exact:true }).click(); await dock.waitFor({ state:"hidden" });
  await page.evaluate(() => { window.monBrowserFixture.holdPlayback = false; window.monBrowserFixture.releasePlayback(); });
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(c => c.command === "close_playback"));
  await nav.getByRole("button",{ name:"概要",exact:true }).click(); await dock.waitFor(); check(await player.count() === 0,"late preparation populated a new dock");
  results.push("late audio preparation after view exit releases its token and cannot populate the new dock");
  await boot(); await page.setViewportSize({ width:360,height:640 });
  const longTitle = "長い会議名".repeat(36);
  await page.evaluate(title => window.monBrowserFixture.setMeetingTitle("meeting-0",title),longTitle); await open(); await play(); await pause();
  await page.waitForFunction(title => document.querySelector(".playback-owner")?.textContent.includes(title),longTitle);
  await page.evaluate(async () => { const { toasts } = await import("/src/toast.tsx"); toasts.show({ key:"fixed-player-long-notice",level:"warning",important:true,message:"長文の注意を確認してください。".repeat(300) }); });
  const notice = page.getByRole("complementary",{ name:"通知",exact:true }); await notice.getByRole("alert").waitFor();
  await page.waitForFunction(() => { const t = document.querySelector(".toast-host").getBoundingClientRect(),d = document.querySelector(".meeting-playback-dock").getBoundingClientRect(),h = document.querySelector(".topbar").getBoundingClientRect(); return t.bottom <= d.top - 8 && t.top >= h.bottom + 8; });
  const b = await bounds(); check(b.dock.bottom - b.dock.top <= b.height * .4 + 1 && !b.dockOverflow && b.main.bottom <= b.dock.top + 1,"long owner broke dock allocation");
  await player.getByRole("button",{ name:"再生",exact:true }).focus();
  check(await player.getByRole("button",{ name:"再生",exact:true }).evaluate(e => { const r = e.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight; }),"long owner hides focused playback action");
  await page.screenshot({ path:"fixed-player-long-owner-toast-360.png" });
  results.push("measured header and variable-height dock bound long notifications without covering playback, even with a 180-character owner");
  return { passed:results.length,results };
}
