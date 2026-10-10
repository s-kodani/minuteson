// Production minutes body in the real tab workspace, synthetic IPC only.
async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  await page.reload(); await page.waitForFunction(() => !!window.__mon);
  await page.evaluate(() => window.__mon.addLongDocument());
  await page.locator(".meeting-name").first().click();
  const tabs = page.getByRole("tablist", { name:"会議詳細", exact:true });
  const transcript = tabs.getByRole("tab", { name:"文字起こし", exact:true }), minutes = tabs.getByRole("tab", { name:"議事録", exact:true });
  await minutes.click();
  const body = page.getByRole("region", { name:"議事録本文", exact:true }); await body.waitFor();
  // Resizing updates CSS immediately, then the media-query listener reparents
  // the panel. Measure the settled application layout, not that event boundary.
  const waitReferenceLayout = () => page.waitForFunction(() => { const panel=document.querySelector('.workspace-panel-shell[data-side="right"]'); return panel?.dataset.narrow===String(matchMedia("(max-width:1119px)").matches); });
  await body.evaluate(e => { window.minutesBody = e; });
  for (const width of [1280,1200,960]) for (const theme of ["light","dark"]) {
    await page.setViewportSize({ width,height:640 });
    await waitReferenceLayout();
    await page.getByRole("banner").getByRole("combobox", { name:"表示テーマ", exact:true }).selectOption(theme);
    await page.waitForFunction(t => document.documentElement.dataset.theme === t, theme);
    for (const end of [false,true]) {
      await body.evaluate((e,end) => { e.scrollTop = end ? e.scrollHeight : 0; },end);
      check(await body.evaluate(e => { const b=e.getBoundingClientRect(), dock=document.querySelector(".meeting-playback-dock").getBoundingClientRect(); return b.height>=120 && b.bottom <= dock.top && document.querySelector(".content").scrollTop === 0 && document.documentElement.scrollHeight <= innerHeight + 1 && document.documentElement.scrollWidth <= innerWidth; }), "minutes body has less than 120px, overlaps chrome or scrolls workspace");
      if (end) check(await body.evaluate(e => e.scrollTop > 0), "long minutes have no body scrollbar");
    }
    await page.screenshot({ path:`detail-minutes-${theme}-${width}.png` });
  }
  results.push("long minutes reserve positive body height at 1280/1200/960 by 640 in both themes and scroll independently of tabs and player");
  await body.evaluate(e => { e.scrollTop = 420; }); const position = await body.evaluate(e => e.scrollTop);
  await transcript.click(); await minutes.click();
  check(await body.evaluate(e => e === window.minutesBody && e.scrollTop === 420) && position === 420, "minutes return remounted body or lost position");
  await body.focus(); await body.press("PageDown");
  await page.waitForFunction(() => document.querySelector(".minutes-body-scroll").scrollTop > 420);
  check(await page.locator(".content").evaluate(e => e.scrollTop) === 0, "minutes keyboard scrolling moved workspace chrome");
  results.push("minutes body node and scroll position survive a transcript round trip; focused PageDown scrolls the body only");
  await page.getByRole("button", { name:"議事録を編集", exact:true }).click();
  for (const width of [1280,1200,960]) for (const theme of ["light","dark"]) {
    await page.setViewportSize({ width,height:640 });
    await waitReferenceLayout();
    await page.getByRole("banner").getByRole("combobox", { name:"表示テーマ", exact:true }).selectOption(theme);
    await page.waitForFunction(t => document.documentElement.dataset.theme === t, theme);
    check(await body.evaluate(e => e.clientHeight >= 80), "rich editing leaves less than 80px of body height");
  }
  const field = page.getByRole("textbox", { name:"議事録ブロック1", exact:true });
  await field.fill("タブ切替でも議事録入力を保持"); await field.evaluate(e => { window.minutesField = e; });
  await page.evaluate(() => { window.__mon.failMinutesSave = true; }); await transcript.click();
  await page.getByRole("alert").filter({ hasText:"編集内容はこの画面に保持" }).waitFor();
  check(await minutes.getAttribute("aria-selected") === "true" && await field.textContent() === "タブ切替でも議事録入力を保持" && await field.evaluate(e => e === window.minutesField), "failed minutes save switched or discarded field");
  results.push("minutes save failure refuses tab switching and retains rich field identity and content");
  await page.evaluate(() => { window.__mon.failMinutesSave = false; });
  await page.getByRole("button", { name:"議事録保存を再試行", exact:true }).first().click();
  await page.waitForFunction(() => window.__mon.drafts.a.payload.blocks[0].content[0].text === "タブ切替でも議事録入力を保持");
  await transcript.click(); await minutes.click();
  check(await field.textContent() === "タブ切替でも議事録入力を保持" && await field.evaluate(e => e === window.minutesField) && await page.getByRole("button", { name:"編集を終了", exact:true }).isVisible(), "minutes return exited editing or replaced field");
  results.push("explicit retry enables switching without exiting rich editing or replacing its field");
  const tools = page.getByText("議事録の設定・生成状況・候補", { exact:true }); await tools.click();
  await page.getByRole("button", { name:"議事録を再生成", exact:true }).click();
  const confirm = page.getByRole("dialog", { name:"議事録の再生成を確認", exact:true });
  if (await confirm.isVisible()) await confirm.getByRole("button", { name:"議事録を再生成する", exact:true }).click();
  await page.waitForFunction(() => window.__mon.jobs.length === 1);
  await tools.click();
  check(await page.getByRole("status").filter({ hasText:"議事録生成：" }).isVisible() && await minutes.getAttribute("aria-selected") === "true", "generation progress vanished when operations collapsed or navigated away");
  await page.evaluate(() => window.__mon.complete());
  await page.waitForFunction(() => !document.querySelector(".minutes-generation-status"));
  check(await minutes.getAttribute("aria-selected") === "true", "completion forced navigation");
  results.push("generation progress remains inside minutes while tools are closed; completion never forces navigation");
  return { passed:results.length, results };
}
