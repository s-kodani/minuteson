// Production workspace, synthetic IPC and silence only; no native recording.
async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const tabs = page.getByRole("tablist", { name: "会議詳細", exact: true });
  const transcript = tabs.getByRole("tab", { name: "文字起こし", exact: true });
  const minutes = tabs.getByRole("tab", { name: "議事録", exact: true });
  const editor = page.getByRole("region", { name: "文字起こし編集", exact: true });
  const body = editor.getByRole("region", { name: "発言本文", exact: true });
  const boot = async () => {
    await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(() => window.monBrowserFixture.setUtterances(Array.from({ length:103 }, (_,i) => ({ id:`speech-${i}`, trackId:"microphone", startUs:i * 1_000_000, endUs:(i + 1) * 1_000_000, recognizedText:`本文${i}です。`, effectiveText:`本文${i}です。`, attributionState:"assigned", speakerIds:[i % 2 ? "speaker-b" : "speaker-a"] }))));
    await page.locator(".meeting-name").first().click();
    await editor.getByRole("button", { name: "編集を開始", exact: true }).waitFor();
  };
  await boot();
  check(await transcript.getAttribute("aria-selected") === "true" && await minutes.getAttribute("aria-selected") === "false" && await page.getByRole("tabpanel").count() === 1, "default tab/accessibility state is invalid");
  const panelNodes = await page.evaluate(() => { window.detailNodes = [document.querySelector(".transcript-card"), document.querySelector(".minutes-card")]; return window.detailNodes.length; });
  check(panelNodes === 2, "panels are not mounted");
  await transcript.focus(); await transcript.press("ArrowRight");
  check(await minutes.evaluate(e => e === document.activeElement) && await transcript.getAttribute("aria-selected") === "true", "arrow key unexpectedly published a tab change");
  await minutes.press("Enter"); await minutes.waitFor({ state: "visible" });
  await page.waitForFunction(() => document.getElementById("meeting-tab-minutes")?.getAttribute("aria-selected") === "true");
  check(await page.getByRole("tabpanel").count() === 1 && await minutes.evaluate(e => e === document.activeElement), "activation did not retain tab focus");
  await minutes.press("Home"); await transcript.press("Space");
  await page.waitForFunction(() => document.getElementById("meeting-tab-transcript")?.getAttribute("aria-selected") === "true");
  check(await page.evaluate(() => window.detailNodes[0] === document.querySelector(".transcript-card") && window.detailNodes[1] === document.querySelector(".minutes-card")), "switching recreated a panel");
  results.push("default selection, labelled panels, manual Arrow/Home activation, Enter/Space and persistent panel identity");

  for (const theme of ["light", "dark"]) for (const width of [1280, 1200, 960]) {
    await page.setViewportSize({ width, height: 640 });
    await page.getByRole("banner").getByRole("combobox", { name: "表示テーマ", exact: true }).selectOption(theme);
    for (const end of [false, true]) {
      await body.evaluate((e, end) => { e.scrollTop = end ? e.scrollHeight : 0; }, end);
      const b = await page.evaluate(() => {
        const rect = s => { const r = document.querySelector(s).getBoundingClientRect(); return { top:r.top, bottom:r.bottom }; };
        return { header:rect(".topbar"), tabs:rect(".meeting-detail-tabs"), main:rect(".content"), body:rect(".utterance-list"), dock:rect(".meeting-playback-dock"), pageHeight:document.documentElement.scrollHeight, pageWidth:document.documentElement.scrollWidth, scroll:document.querySelector(".content").scrollTop, bodyScroll:document.querySelector(".utterance-list").scrollTop, height:innerHeight, width:innerWidth };
      });
      check(b.header.top === 0 && b.tabs.top === b.header.bottom && b.main.top === b.tabs.bottom && b.body.bottom <= b.dock.top && b.body.bottom > b.body.top && b.main.bottom <= b.dock.top && b.dock.bottom <= b.height + 1 && b.pageHeight <= b.height + 1 && b.pageWidth <= b.width && b.scroll === 0 && (!end || b.bodyScroll > 0), `body-only scroll invalid ${JSON.stringify(b)}`);
    }
    await page.screenshot({ path:`detail-tabs-${theme}-${width}.png` });
  }
  results.push("light/dark at 1280/1200/960 by 640: only utterance body scrolls; header, tabs and dock reserve non-overlapping space");
  await body.evaluate(e => { e.scrollTop = 530; }); const position = await body.evaluate(e => e.scrollTop);
  await minutes.click(); await transcript.click();
  check(await body.evaluate(e => e.scrollTop) === position, "tab return lost body position");
  await body.focus(); await body.press("PageDown");
  await page.waitForFunction(p => document.querySelector(".utterance-list").scrollTop > p, position);
  check(await body.evaluate(e => e.scrollTop) > position && await page.locator(".content").evaluate(e => e.scrollTop) === 0, "PageDown scrolled workspace chrome");
  results.push("tab return preserves body position and focused PageDown scrolls the body only");

  await editor.getByRole("button", { name: "編集を開始", exact: true }).click();
  const row = editor.getByRole("article", { name: "00:00:00の発言行", exact: true });
  await row.getByRole("button", { name: "00:00:00の発言を編集", exact: true }).click();
  const input = row.getByRole("textbox", { name: "00:00:00の発言", exact: true });
  await input.fill("タブ切替後も入力を保持します。");
  await input.evaluate(e => { window.detailInput = e; e.setSelectionRange(2, 5); });
  await page.evaluate(() => { window.monBrowserFixture.failSave = true; }); await minutes.click();
  await page.getByRole("alert").filter({ hasText:"編集内容はこの画面に保持" }).waitFor();
  check(await transcript.getAttribute("aria-selected") === "true" && await input.inputValue() === "タブ切替後も入力を保持します。" && await input.evaluate(e => e === window.detailInput), "failed save switched or discarded input");
  results.push("failed save refuses tab activation and retains original panel and input DOM");
  await page.evaluate(() => { window.monBrowserFixture.failSave = false; });
  await editor.getByRole("button", { name:"保存を再試行", exact:true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.draft.payload.utterances.some(u => u.effective_text === "タブ切替後も入力を保持します。"));
  await minutes.click(); await transcript.click();
  check(await input.inputValue() === "タブ切替後も入力を保持します。" && await input.evaluate(e => e === window.detailInput), "successful flush recreated input or exited editing");
  results.push("explicit save retry permits later switching without recreating the editor or exiting edit mode");
  await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs = 500; });
  await input.fill("保存完了を待ってタブを切り替えます。"); await minutes.click();
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(c => c.command === "save_transcript_draft" && c.args.payload.utterances.some(u => u.effective_text === "保存完了を待ってタブを切り替えます。")));
  check(await transcript.getAttribute("aria-selected") === "true" && await minutes.isDisabled(), "tab switched before native save acknowledgement");
  await page.waitForFunction(() => document.getElementById("meeting-tab-minutes").getAttribute("aria-selected") === "true");
  await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs = 0; }); await transcript.click();
  check(await input.inputValue() === "保存完了を待ってタブを切り替えます。" && await input.evaluate(e => e === window.detailInput), "delayed tab switch replaced confirmed input");
  results.push("delayed save keeps the original tab and disables further activation until acknowledgement; return retains input identity");
  await boot(); await editor.getByRole("button", { name:"編集を開始", exact:true }).click();
  await row.getByRole("button", { name:"発言を手動分割", exact:true }).click();
  const split = page.getByRole("dialog", { name:"発言の手動分割", exact:true }); await split.waitFor();
  await page.keyboard.press("Escape"); await split.waitFor({ state:"hidden" });
  check(await transcript.getAttribute("aria-selected") === "true", "dialog cancellation changed selected tab");
  results.push("split modal remains exclusive and cancellation returns to the original tab");
  await page.evaluate(async () => {
    const snapshot = { revision:5, phase:"recording", elapsedMs:83000, microphoneConnected:true, systemAudioConnected:true, meetingId:"meeting-0", sessionId:"synthetic-tab-session", errorMessage:null };
    window.monBrowserFixture.recording = snapshot;
    await window.__TAURI_INTERNALS__.invoke("plugin:event|emit", { event:"session-changed", payload:snapshot });
  });
  await minutes.click(); await page.getByRole("banner").getByRole("button", { name:"録音を終了", exact:true }).waitFor();
  await minutes.focus(); await page.keyboard.press("Control+f");
  await page.waitForFunction(() => document.activeElement?.matches(".transcript-search input[type=search]"));
  check(await transcript.getAttribute("aria-selected") === "true" && await page.getByRole("banner").getByRole("button", { name:"録音を終了", exact:true }).isVisible(), "Find did not return to transcript or recording end became hidden");
  check(await page.evaluate(() => !window.monBrowserFixture.calls.some(c => /^(start_recording|stop_recording|request_recording_confirmation)$/u.test(c.command))), "tab/search operations mutated recording");
  results.push("recording end remains available on both tabs; Find returns to transcript after the save barrier without changing recording");
  await minutes.click(); await page.getByRole("navigation", { name:"メインナビゲーション" }).getByRole("button", { name:"会議一覧", exact:true }).click();
  await tabs.waitFor({ state:"hidden" }); await page.locator(".meeting-name").nth(1).click();
  check(await transcript.getAttribute("aria-selected") === "true" && (await page.locator(".playback-owner").innerText()).includes("会議B"), "new meeting inherited old tab or owner");
  results.push("meeting list navigation leaves detail; another meeting starts on transcript with the correct playback owner");
  return { passed:results.length, results };
}
