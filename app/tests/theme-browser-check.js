async (page, capturePath = path => path) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const select = () => page.getByRole("combobox", { name: "表示テーマ", exact: true });
  const waitTheme = theme => page.waitForFunction(value => document.documentElement.dataset.theme === value, theme);
  const reset = async () => { await page.evaluate(() => window.themeFixture.reset()); await page.reload(); await select().waitFor(); await page.waitForFunction(() => !document.querySelector(".theme-quick select")?.disabled); };
  const count = () => page.evaluate(() => window.themeFixture.calls.filter(c => c.command === "save_ui_preferences").length);
  await reset(); await page.emulateMedia({ colorScheme: "light" }); await waitTheme("light");
  await page.emulateMedia({ colorScheme: "dark" }); await waitTheme("dark");
  check(await count() === 0, "OS follow wrote settings");
  await select().selectOption("light"); await waitTheme("light");
  await page.emulateMedia({ colorScheme: "dark" }); await waitTheme("light");
  await select().selectOption("dark"); await waitTheme("dark");
  await page.emulateMedia({ colorScheme: "light" }); await waitTheme("dark");
  await page.reload(); await waitTheme("dark"); check(await select().inputValue() === "dark", "saved theme not reloaded");
  results.push("three choices, live OS follow without writes, explicit override and reload persistence");

  await page.getByRole("button", { name: "開く", exact: true }).click();
  const editor = page.getByRole("region", { name: "文字起こし編集", exact: true });
  const text = editor.getByRole("textbox", { name: "00:00:00の発言", exact: true });
  await text.fill("編集中の点検日を保持します。"); await editor.getByRole("searchbox", { name: "発言を検索", exact: true }).fill("点検");
  await text.evaluate(node => { window.themeFixture.editorNode = node; node.setSelectionRange(2, 5); });
  const reads = await page.evaluate(() => [window.audioDeletionFixture.analysisReads, window.audioDeletionFixture.draftReads]);
  await editor.getByRole("button", { name: "音声を再生", exact: true }).click();
  const audio = page.locator(".audio-player"); await audio.getByRole("button", { name: "一時停止", exact: true }).click();
  const seek = audio.getByRole("slider", { name: "発言内の再生位置", exact: true });
  await seek.focus(); await seek.press("Home");
  for (let step = 0; step < 5; step++) await seek.press("ArrowRight");
  await audio.getByRole("combobox", { name: "速度", exact: true }).selectOption("1.5");
  for (const theme of ["light", "dark"]) {
    await select().selectOption(theme); await waitTheme(theme);
    check(await text.inputValue() === "編集中の点検日を保持します。" && await text.evaluate(node => node === window.themeFixture.editorNode), "theme remounted or reset editor");
    check(await text.evaluate(node => node.selectionStart === 2 && node.selectionEnd === 5), "theme lost text selection");
    check(await editor.getByRole("searchbox", { name: "発言を検索", exact: true }).inputValue() === "点検", "theme reset search");
    check(await seek.inputValue() === "8000" && await audio.getByRole("combobox", { name: "速度", exact: true }).inputValue() === "1.5", "theme reset playback position/rate");
    check(JSON.stringify(reads) === JSON.stringify(await page.evaluate(() => [window.audioDeletionFixture.analysisReads, window.audioDeletionFixture.draftReads])), "theme reloaded document heads");
    await page.screenshot({ path: `editor-${theme}.png`, fullPage: true });
  }
  results.push("mounted production transcript edit/search/selection and actual silent AudioContext playback position/rate retained");

  await page.getByRole("button", { name: "議事録を編集", exact: true }).click();
  const rich = page.getByRole("textbox", { name: "議事録ブロック2", exact: true });
  await page.evaluate(() => { window.themeFixture.minutesSaveDelay = true; });
  await rich.fill("編集中の点検日と議事録を保持します。");
  await rich.evaluate(node => {
    window.themeFixture.richNode = node;
    const range = document.createRange(); range.setStart(node.firstChild, 2); range.setEnd(node.firstChild, 5);
    const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
    node.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  });
  for (const theme of ["light", "dark"]) {
    await select().selectOption(theme); await waitTheme(theme);
    check(await rich.textContent() === "編集中の点検日と議事録を保持します。" && await rich.evaluate(node => node === window.themeFixture.richNode), "theme remounted or reset rich editor");
    check(await rich.evaluate(node => { const s = getSelection(); return s.rangeCount === 1 && s.getRangeAt(0).startContainer === node.firstChild && s.getRangeAt(0).startOffset === 2 && s.getRangeAt(0).endOffset === 5; }), "theme lost rich selection");
    check(await page.getByRole("button", { name: "編集を終了", exact: true }).isVisible(), "theme exited rich editing");
    check(await page.evaluate(() => window.themeFixture.savedMinutes().payload.blocks[1].content[0].text) === "保存済みの点検日メモです。", "theme discarded or forced incomplete rich save");
    await rich.locator("..").screenshot({ path: capturePath(`rich-${theme}.png`) });
  }
  await page.waitForFunction(() => !!window.themeFixture.releaseMinutesSave);
  await page.evaluate(() => { window.themeFixture.minutesSaveDelay = false; window.themeFixture.releaseMinutesSave(); });
  await page.waitForFunction(() => window.themeFixture.savedMinutes().payload.blocks[1].content[0].text === "編集中の点検日と議事録を保持します。");
  results.push("production rich text stays mounted with unsaved content, range and editing state across both themes; normal delayed autosave completes unchanged");

  const contrast = [];
  for (const theme of ["light", "dark"]) {
    await select().selectOption(theme); await waitTheme(theme);
    const ratios = await page.evaluate(() => {
      const style = getComputedStyle(document.documentElement);
      const luminance = name => {
        const hex = style.getPropertyValue(name).trim().slice(1);
        const rgb = [0, 2, 4].map(offset => parseInt(hex.slice(offset, offset + 2), 16) / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4);
        return rgb[0] * .2126 + rgb[1] * .7152 + rgb[2] * .0722;
      };
      return [["--text", "--surface", 4.5], ["--muted", "--surface", 4.5], ["--secondary-text", "--surface", 4.5], ["--on-primary", "--primary", 4.5], ["--accent", "--selection", 4.5], ["--danger-text", "--surface", 4.5], ["--on-danger", "--danger-button", 4.5], ["--success-text", "--success-surface", 4.5], ["--warning-text", "--warning-surface", 4.5], ["--info-text", "--info-surface", 4.5], ["--focus", "--surface", 3], ["--recording", "--surface", 3]].map(([fg, bg, minimum]) => {
        const a = luminance(fg), b = luminance(bg);
        return { fg, bg, minimum, ratio: (Math.max(a, b) + .05) / (Math.min(a, b) + .05) };
      });
    });
    for (const pair of ratios) check(Number.isFinite(pair.ratio) && pair.ratio >= pair.minimum, `${theme} ${pair.fg}/${pair.bg} contrast ${pair.ratio}`);
    contrast.push({ theme, ratios });
    await text.focus();
    check(await text.evaluate(node => getComputedStyle(node).outlineStyle !== "none" && parseFloat(getComputedStyle(node).outlineWidth) >= 2), "editor focus indicator invisible");
    for (const width of [960, 1280]) {
      await page.setViewportSize({ width, height: 800 });
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme} main horizontal overflow at ${width}`);
    }
  }
  results.push({ case: "12 token pairs per theme meet text 4.5 or indicator 3 contrast; visible editor focus; no horizontal overflow at 960/1280", contrast });

  // Dialogs are modal: OS follow must recolor them without dismissing state.
  await select().selectOption("system"); await waitTheme("light");
  await page.getByRole("button", { name: "既存会議の音声だけを削除", exact: true }).click();
  const dialog = page.getByRole("dialog"); await dialog.waitFor();
  for (const theme of ["light", "dark"]) {
    await page.emulateMedia({ colorScheme: theme }); await waitTheme(theme);
    const colors = await dialog.evaluate(node => ({ background: getComputedStyle(node).backgroundColor, surface: getComputedStyle(document.documentElement).getPropertyValue("--surface").trim() }));
    check(colors.background === (theme === "dark" ? "rgb(17, 24, 39)" : "rgb(255, 255, 255)"), "dialog has unthemed surface");
    await page.screenshot({ path: `dialog-${theme}.png` });
  }
  await page.keyboard.press("Escape"); await dialog.waitFor({ state: "hidden" });
  check(await page.evaluate(() => window.audioDeletionFixture.deleted) === 0, "theme test executed deletion");
  results.push("light/dark modal surface, live OS recolor, Escape without deletion");

  await page.getByRole("button", { name: "設定", exact: true }).click();
  const settings = page.getByRole("region", { name: "表示設定", exact: true });
  const themeError = () => page.getByRole("complementary", { name: "通知", exact: true }).getByRole("alert").filter({ hasText: "表示設定の読込・保存" });
  await settings.getByRole("combobox", { name: "テーマ", exact: true }).selectOption("dark"); await waitTheme("dark");
  check(await select().inputValue() === "dark", "settings and header diverged");
  const mini = await page.context().newPage();
  try {
    await mini.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
      const response = await route.fetch();
      const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/theme-browser-fixture.js");</script>');
      await route.fulfill({ response, body });
    });
    await mini.goto("http://127.0.0.1:1420/?window=recorder"); await mini.waitForFunction(() => document.documentElement.dataset.theme === "dark");
    await mini.setViewportSize({ width: 380, height: 130 });
    await mini.screenshot({ path: capturePath("mini-dark.png") });
    await page.evaluate(() => { window.themeFixture.suppressEvents = true; });
    await select().selectOption("light"); await waitTheme("light");
    await mini.waitForFunction(() => document.documentElement.dataset.theme === "light");
    check(await mini.evaluate(() => window.themeFixture.calls.filter(c => c.command === "save_ui_preferences").length) === 0, "mini wrote theme");
    await mini.screenshot({ path: capturePath("mini-light.png") });
  } finally { await mini.close(); }
  results.push("settings/header sync; separate recorder window reloads saved theme without events or writes");

  await page.evaluate(() => { window.themeFixture.mode = "fail"; });
  await select().selectOption("dark"); await themeError().filter({ hasText: "保存できません" }).waitFor();
  check(await settings.getByLabel("表示設定の読込・保存の要対応状態", { exact: true }).isVisible(), "failed theme has no retained issue");
  await waitTheme("light"); check(await select().inputValue() === "dark", "failed selection lost");
  const failed = await count();
  await page.evaluate(async () => { await window.themeFixture.emitHint({ theme: "dark", lockVersion: 99999 }); });
  await waitTheme("light"); check(await count() === failed, "event retried write");
  await page.evaluate(() => { window.themeFixture.mode = "success"; });
  await settings.getByRole("button", { name: "テーマの保存を再試行", exact: true }).click(); await waitTheme("dark");
  results.push("failed save retains choice and confirmed colors; event hint never grants theme or resaves; explicit retry");

  await page.evaluate(() => { window.themeFixture.mode = "lost"; });
  const beforeLost = await count(); await select().selectOption("light"); await waitTheme("light");
  check(await count() === beforeLost + 1, "lost acknowledgement duplicated save");
  await page.evaluate(() => { window.themeFixture.mode = "lostUnreadable"; });
  await select().selectOption("dark"); await themeError().filter({ hasText: "保存結果を確認できません" }).waitFor();
  check(await select().isDisabled(), "uncertain write can be retried");
  const uncertain = await count(); await page.evaluate(() => { window.themeFixture.readFail = false; });
  await settings.getByRole("button", { name: "保存済み表示設定を再読込", exact: true }).click(); await waitTheme("dark");
  check(await count() === uncertain, "uncertain commit was rewritten");
  results.push("lost committed save recovers by read; unknown commit blocks write until confirmed reload");

  await select().selectOption("light"); await themeError().filter({ hasText: "保存結果を確認できません" }).waitFor();
  const beforePoll = await count(); await page.evaluate(() => { window.themeFixture.readFail = false; });
  await waitTheme("light");
  check(await select().inputValue() === "light" && await themeError().count() === 0 && await settings.getByRole("button", { name: "テーマの保存を再試行", exact: true }).count() === 0, "authoritative polling left false uncertainty or retry");
  check(await count() === beforePoll, "polling rewrote the committed theme");
  results.push("later authoritative poll resolves lost commit and clears stale uncertainty/choice without events, manual reload or resave");

  await page.evaluate(async () => { window.audioDeletionFixture.blocked = true; await window.dataFixture.changed({}); });
  await page.waitForFunction(() => document.querySelector(".theme-quick select")?.disabled);
  check(await settings.getByRole("combobox", { name: "テーマ", exact: true }).isDisabled(), "data write fence failed to disable theme save");
  results.push("data-maintenance write fence disables both theme writers while saved colors remain");
  return results;
}
