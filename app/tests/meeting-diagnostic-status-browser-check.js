// Synthetic IPC checks for the product list/detail. No native files or recordings.
async (page, capturePath) => {
  const check = (condition, message) => { if (!condition) throw Error(message); };
  const results = [];
  const row = () => page.getByRole("row").filter({ hasText: "議事録UI確認" });
  const notice = () => page.locator(".meeting-detail-content > .workspace-warning");
  const scenario = async (reasons, label, { waitForTranscript = true } = {}) => {
    await page.setViewportSize({ width: 1360, height: 900 });
    await page.goto("http://127.0.0.1:1420/?resilience=1");
    await page.getByText(/会議一覧を取得できませんでした/).waitFor();
    await page.evaluate(reasons => {
      Object.assign(window.__mon.health, {
        listError: false, draftReadFailure: false, modelListingFailure: false,
        diagnostics: reasons.map(reason => ({ meetingId: "m1", reason, message: `合成診断: ${reason}` })),
      });
    }, reasons);
    await page.getByRole("button", { name: "再読込", exact: true }).click();
    await row().locator(".state-pill").getByText(label, { exact: true }).waitFor();
    check(await row().getByText("要確認", { exact: true }).count() === 0, "vague meeting status remains");
    // The resilience fixture initially fails unrelated model/list reads. Close
    // those notices through the product UI before capturing the status change.
    const closeError = page.locator('.mon-notification-stack button[aria-label="エラー通知を閉じる"]:visible');
    for (let i = 0; i < 5 && await closeError.count(); i++) {
      await closeError.first().click();
      await page.clock.runFor(500);
    }
    if (reasons.length === 1 && reasons[0] === "audio_missing") {
      await page.screenshot({ path: capturePath("01-meeting-list-audio-missing.png") });
    }
    await row().getByRole("button", { name: "議事録UI確認", exact: true }).click();
    await notice().locator("strong").getByText(label, { exact: true }).waitFor();
    if (waitForTranscript) await page.locator(".utterance-list").waitFor();
    if (reasons.length === 1 && reasons[0] === "audio_missing") {
      await page.getByRole("button", { name: "00:00:01の発言を編集", exact: true }).waitFor();
      await page.screenshot({ path: capturePath("02-meeting-detail-audio-missing.png") });
    }
  };

  for (const [reason, label] of [["audio_missing", "音声なし"], ["audio_unreadable", "音声を利用できません"]]) {
    await scenario([reason], label);
    await notice().getByText(/文字起こし・議事録は引き続き閲覧・編集できます/).waitFor();
    const edit = page.getByRole("button", { name: "00:00:01の発言を編集", exact: true });
    await edit.click();
    const utterance = page.locator('[data-utterance-id="u"]');
    await utterance.getByRole("textbox", { name: "00:00:01の発言", exact: true }).fill("音声がなくても本文を保存できます。");
    await utterance.getByRole("button", { name: "保存", exact: true }).click();
    await edit.filter({ hasText: "音声がなくても本文を保存できます。" }).waitFor();
    check(await page.evaluate(() => window.__mon.intervalCalls.every(call => call.cmd !== "get_meeting_audio_summary")), "unavailable audio was accessed");
    await page.getByRole("tab", { name: "議事録", exact: true }).click();
    const minutes = page.getByRole("tabpanel", { name: "議事録", exact: true });
    await minutes.getByRole("button", { name: "議事録を編集", exact: true }).click();
    check(await minutes.getByRole("textbox", { name: "議事録のMarkdown本文", exact: true }).isEnabled(), "audio-only fault disabled minutes editing");
    await minutes.getByRole("button", { name: "議事録のプレビューを表示", exact: true }).click();
    results.push(`${reason}: clear audio status, no playback access, transcript save and minutes editing remain available`);
  }

  await scenario(["audio_missing", "meeting_row_invalid"], "会議データを利用できません");
  await notice().getByText(/修復する機能はありません.*会議の操作メニューから「削除」/).waitFor();
  check(!/閲覧・編集できます/.test(await notice().innerText()), "protected meeting also promised document editing");
  const edit = page.getByRole("button", { name: "00:00:01の発言を編集", exact: true });
  await page.locator(".utterance-list").waitFor();
  check(await edit.count() === 0 || await edit.isDisabled(), "unreadable meeting allowed transcript editing");
  await page.getByRole("button", { name: "会議の操作", exact: true }).click();
  await page.getByRole("menuitem", { name: "議事録UI確認を削除", exact: true }).waitFor();
  await page.keyboard.press("Escape");
  await scenario(["meeting_row_invalid"], "会議データを利用できません");
  for (const width of [1360, 640]) {
    await page.setViewportSize({ width, height: 900 });
    check(await notice().evaluate(element => {
      const bounds = element.getBoundingClientRect(), content = element.closest(".meeting-detail-content"), frame = content.getBoundingClientRect();
      const inset = parseFloat(getComputedStyle(content).getPropertyValue("--tab-content-padding-inline"));
      const style = getComputedStyle(element);
      return element.classList.contains("callout") && style.backgroundColor !== "rgba(0, 0, 0, 0)" && parseFloat(style.borderLeftWidth) > 0 &&
        [style.paddingTop, style.paddingRight, style.paddingBottom, style.paddingLeft].every(value => parseFloat(value) >= 16) &&
        parseFloat(style.rowGap) >= 8 && parseFloat(style.marginTop) >= 12 &&
        Math.abs(bounds.left - frame.left - inset) <= 1 && Math.abs(bounds.right - frame.right + inset) <= 1 && document.documentElement.scrollWidth <= innerWidth;
    }), `diagnostic notice overflowed or lost workspace alignment at ${width}px`);
    await page.screenshot({ path: capturePath(`03-meeting-data-unavailable-${width}.png`) });
  }
  results.push("meeting data fault takes priority, explains blocked editing and deletion, and fits both workspace widths");

  for (const [reason, label] of [["deletion_pending", "会議の削除が未完了"], ["audio_deletion_pending", "音声の削除が未完了"]]) {
    await scenario([reason], label);
    await notice().getByText(/この画面から.*できません/).waitFor();
    check(!/メニュー|再試行|続行/.test(await notice().innerText()), "pending deletion offered an unavailable continuation");
    results.push(`${reason}: unfinished deletion is explicit without offering an unavailable continuation`);
  }

  await scenario(["salvage_minutes"], "議事録を復元できませんでした");
  await notice().getByText(/復元できたデータは引き続き利用できます/).waitFor();
  await page.getByRole("button", { name: "00:00:01の発言を編集", exact: true }).click();
  check(await page.getByRole("textbox", { name: "00:00:01の発言", exact: true }).isEnabled(), "past partial salvage blocked current document editing");
  results.push("partial salvage describes the past recovery result while retaining current document editing");

  const lazyModule = url => ["/src/transcript.tsx", "/src/analysis.tsx"].includes(url.pathname);
  const { promise: heldModules, resolve: releaseModules } = Promise.withResolvers();
  const moduleContinuations = [];
  const holdModules = route => {
    const continuation = heldModules.then(() => route.continue());
    moduleContinuations.push(continuation);
    return continuation;
  };
  await page.route(lazyModule, holdModules);
  try {
    await Promise.all([
      page.waitForRequest(request => new URL(request.url()).pathname === "/src/transcript.tsx"),
      page.waitForRequest(request => new URL(request.url()).pathname === "/src/analysis.tsx"),
      scenario(["meeting_row_invalid"], "会議データを利用できません", { waitForTranscript: false }),
    ]);
    const loaders = page.locator(".meeting-detail-content .deferred-panel-loading:visible");
    await loaders.first().waitFor();
    check(await loaders.count() === 2, "cold detail did not display both real Suspense fallbacks");
    for (const width of [1360, 640]) {
      await page.setViewportSize({ width, height: 900 });
      check(await loaders.evaluateAll(elements => elements.every(element => {
        const content = element.closest(".meeting-detail-content"), frame = content.getBoundingClientRect();
        const inset = parseFloat(getComputedStyle(content).getPropertyValue("--tab-content-padding-inline"));
        const range = document.createRange(); range.selectNodeContents(element);
        const text = range.getBoundingClientRect(), style = getComputedStyle(element);
        const heading = content.querySelector(".transcript-primary-heading h2").getBoundingClientRect();
        return Math.abs(text.left - frame.left - inset) <= 1 && parseFloat(style.paddingRight) === inset && text.right <= frame.right - inset &&
          Math.abs(text.left - heading.left) <= 1 &&
          parseFloat(style.paddingTop) >= 16 && parseFloat(style.paddingBottom) >= 16;
      })), `cold detail loader did not match the content gutters at ${width}px`);
      await page.screenshot({ path: capturePath(`04-meeting-detail-loading-${width}.png`) });
    }
  } finally {
    releaseModules();
    await Promise.all(moduleContinuations);
    await page.unroute(lazyModule, holdModules);
  }
  await page.locator(".utterance-list").waitFor();
  await page.waitForFunction(() => !document.querySelector(".meeting-detail-content .deferred-panel-loading"));

  const settingsModule = url => url.pathname === "/src/model-inspection.tsx";
  const { promise: heldSettings, resolve: releaseSettings } = Promise.withResolvers();
  const settingsContinuations = [];
  const holdSettings = route => {
    const continuation = heldSettings.then(() => route.continue());
    settingsContinuations.push(continuation);
    return continuation;
  };
  await page.route(settingsModule, holdSettings);
  try {
    await Promise.all([
      page.waitForRequest(request => settingsModule(new URL(request.url()))),
      page.locator(".sidebar").getByRole("button", { name: "設定", exact: true }).click(),
    ]);
    const loading = page.locator("main.content .deferred-panel-loading:visible");
    await loading.waitFor();
    check(await loading.evaluate(element => {
      const content = element.closest("main"), frame = content.getBoundingClientRect(), style = getComputedStyle(element);
      return !content.classList.contains("meeting-detail-content") && parseFloat(style.paddingLeft) === 0 &&
        Math.abs(element.getBoundingClientRect().left - frame.left - parseFloat(getComputedStyle(content).paddingLeft)) <= 1;
    }), "detail loader padding was also applied inside the settings content padding");
  } finally {
    releaseSettings();
    await Promise.all(settingsContinuations);
    await page.unroute(settingsModule, holdSettings);
  }
  await page.getByRole("region", { name: "ローカルAI詳細設定", exact: true }).waitFor();
  results.push("held real transcript/analysis modules show padded fallbacks at both widths; release loads the documents and settings retains its own gutters");
  return { passed: results.length, results };
}
