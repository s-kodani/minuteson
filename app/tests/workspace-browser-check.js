// Production main layout and navigation with synthetic session state only.
async page => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
  const header = page.getByRole("banner"), workspace = page.getByRole("main", { name: "会議ワークスペース", exact: true });
  const recording = header.getByLabel("共通録音状態", { exact: true });
  const nav = page.getByRole("navigation", { name: "メインナビゲーション", exact: true });
  const open = () => header.getByRole("button", { name: "録音画面へ", exact: true });
  await nav.getByRole("button", { name: "設定", exact: true }).waitFor();
  await page.waitForFunction(() => !document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
  const bounds = async () => page.evaluate(() => {
    const h = document.querySelector(".topbar").getBoundingClientRect(), m = document.querySelector(".content").getBoundingClientRect(), n = document.querySelector(".sidebar").getBoundingClientRect();
    return { headerTop: h.top, headerBottom: h.bottom, mainTop: m.top, mainBottom: m.bottom, navWidth: n.width, height: innerHeight, width: innerWidth, pageWidth: document.documentElement.scrollWidth, pageHeight: document.documentElement.scrollHeight, scroll: document.querySelector(".content").scrollTop, pageScroll: scrollY };
  });
  for (const width of [1280, 1200, 1120, 1119, 960]) {
    await page.setViewportSize({ width, height: 640 });
    await workspace.evaluate(e => { e.scrollTop = e.scrollHeight; });
    const b = await bounds();
    check(b.navWidth === (width >= 1280 ? 220 : 64), `navigation width mismatch: ${JSON.stringify(b)}`);
    check(b.headerTop === 0 && b.mainTop === b.headerBottom && b.mainBottom <= b.height + 1 && b.pageHeight <= b.height + 1 && b.pageWidth <= width && b.scroll > 0 && b.pageScroll === 0, `workspace obscures header or scrolls the page: ${JSON.stringify(b)}`);
    check(await nav.getByRole("button", { name: "設定", exact: true }).getAttribute("title") === "設定", "collapsed navigation lost its name or tooltip");
  }
  results.push("1280/1200/1120/1119/960px reserve the header and navigation outside the body scrollport, with accessible collapsed navigation");
  await workspace.focus(); await workspace.press("Home"); await workspace.press("PageDown");
  await page.waitForFunction(() => document.querySelector(".content").scrollTop > 100);
  check((await bounds()).headerTop === 0 && (await bounds()).pageScroll === 0, "keyboard scroll hid the header");
  results.push("keyboard PageDown scrolls the focused workspace without moving the header");

  const session = { revision: 5, phase: "recording", elapsedMs: 8341000, microphoneConnected: true, systemAudioConnected: true, meetingId: "meeting-1", sessionId: "synthetic-header-session", errorMessage: null };
  const update = async s => page.evaluate(async snapshot => {
    window.monBrowserFixture.recording = snapshot;
    await window.__TAURI_INTERNALS__.invoke("plugin:event|emit", { event: "session-changed", payload: snapshot });
  }, s);
  await update(session); await open().waitFor();
  for (const screen of ["設定", "会議一覧"]) {
    await nav.getByRole("button", { name: screen, exact: true }).click();
    await page.waitForFunction(name => document.querySelector('.nav-item[aria-current="page"]')?.getAttribute("aria-label") === name, screen);
    check((await recording.innerText()).includes("録音中") && (await recording.innerText()).includes("02:19:01") && await open().isVisible(), `${screen} lost shared recording state`);
  }
  await nav.getByRole("button", { name: "設定", exact: true }).click(); await open().click();
  await page.locator(".recording-workspace-title").waitFor();
  check(await page.locator(".recording-workspace-title").textContent() === "会議B", "recording link did not select its meeting");
  await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "録音状態");
  results.push("every screen shows the same canonical recording time; the link returns to the recording meeting and focuses its status without stopping");
  for (const phase of ["preparing", "stopping", "saving"]) {
    await update({ ...session, revision: ++session.revision, phase }); await open().waitFor();
    await page.waitForFunction(p => document.querySelector('.header-status')?.textContent.includes(p), { preparing: "準備中", stopping: "停止中", saving: "保存中" }[phase]);
    check(!(await recording.innerText()).includes("録音中"), `${phase} was labelled recording`);
  }
  await update({ ...session, revision: ++session.revision, phase: "idle", sessionId: null, meetingId: null }); await open().waitFor({ state: "hidden" });
  check((await recording.innerText()).includes("待機中"), "idle header retained active recording");
  results.push("preparation/stopping/saving retain their exact phases and idle removes the recording link");

  await update({ ...session, revision: ++session.revision });
  await nav.getByRole("button",{name:"会議一覧",exact:true}).click();
  await page.locator(".meeting-name").first().click();
  const editor = page.getByRole("region", { name: "文字起こし編集", exact: true }), text = editor.getByRole("textbox", { name: "00:00:00の発言", exact: true });
  await editor.getByRole("button", { name: "編集を開始", exact: true }).click();
  await editor.getByRole("button", { name: "00:00:00の発言を編集", exact: true }).click();
  await page.evaluate(() => { window.monBrowserFixture.failSave = true; }); await text.fill("未保存の点検日を保持します。");
  await text.evaluate(e => { window.monBrowserFixture.headerEditNode = e; }); await open().click();
  await page.getByRole("alert").filter({ hasText: "編集内容はこの画面に保持" }).waitFor();
  check(await text.inputValue() === "未保存の点検日を保持します。" && await text.evaluate(e => e === window.monBrowserFixture.headerEditNode) && await page.locator("#rename-title").inputValue() === "会議A", "failed flush discarded input or navigated to another meeting");
  results.push("the recording link waits for edited content; failed saving preserves the mounted input and current meeting");
  await page.evaluate(() => { window.monBrowserFixture.failSave = false; });
  await editor.getByRole("button", { name: "保存を再試行", exact: true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.draft.payload.utterances.some(u => u.effective_text === "未保存の点検日を保持します。"));
  await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs = 700; });
  await text.fill("遅い保存中も点検日を保持します。"); await open().click();
  await update({ ...session, revision: ++session.revision, sessionId: "different-synthetic-session" });
  await page.getByRole("alert").filter({ hasText: "録音の対象が変更" }).waitFor();
  check(await page.locator("#rename-title").inputValue() === "会議A" && await text.inputValue() === "遅い保存中も点検日を保持します。", "changed recording was adopted after a delayed flush");
  results.push("a recording session changed during edit flushing refuses the old navigation target without losing input");
  await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs = 0; });
  await page.setViewportSize({ width: 960, height: 640 });
  for (const theme of ["light", "dark"]) {
    await header.getByRole("combobox", { name: "表示テーマ", exact: true }).selectOption(theme);
    await page.waitForFunction(t => document.documentElement.dataset.theme === t, theme);
    await workspace.evaluate(e => { e.scrollTop = e.scrollHeight; });
    await page.screenshot({ path: `workspace-header-${theme}-960.png` });
    check((await bounds()).headerTop === 0 && await open().isVisible(), `${theme} hid the header at the end of content`);
  }
  const changes = await page.evaluate(() => window.monBrowserFixture.calls.filter(c => /^(start_recording|stop_recording|request_recording_confirmation|decide_recording_confirmation|finish_main_close)$/u.test(c.command)));
  check(changes.length === 0, "header navigation changed recording or closed the app");
  results.push("both themes keep the header visible at the end of content; all navigation emits no recording stop/start/quit commands");
  return { passed: results.length, results };
}
