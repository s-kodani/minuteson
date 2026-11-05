// Production UI, synthetic closed recordings only. No native files or audio.
async page => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,
      '<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  try {
    await page.goto("http://127.0.0.1:1420/");
    await page.waitForFunction(() => !!window.closeFixture);
    const title = await page.evaluate(() => {
      const f = window.closeFixture;
      f.archived.rows = Array.from({ length: 18 }, (_, i) => f.newArchivedRow({
        title: `保存された会議${i + 1}の長い通知履歴を確認するための日本語タイトル`,
        count: 0, both: true, loss: 12, historySaveIssue: "failed",
      }));
      f.recordChanged({ phase: "idle" });
      return window.dataFixture.meetings[0].title;
    });
    await page.getByRole("button", { name: title, exact: true }).click();
    const history = page.locator(".archived-input-notices details");
    await history.locator("summary").click();
    await page.waitForFunction(() => document.querySelectorAll(".archived-input-notices li").length === 16);
    let passed = 0;
    for (const theme of ["light", "dark"]) for (const width of [1280, 960, 360]) {
      await page.setViewportSize({ width, height: 700 });
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      const section = page.locator(".archived-input-notices");
      check(await section.evaluate(e => getComputedStyle(e).overflowY === "visible"
        && getComputedStyle(e).maxHeight === "none" && e.scrollHeight <= e.clientHeight + 1), "nested history scrollport remains");
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "history overflows horizontally");
      const last = history.locator("li").last().getByRole("button", { name: "会議を開く", exact: true });
      await last.scrollIntoViewIfNeeded();
      check(await last.evaluate(e => { const r=e.getBoundingClientRect(); return r.top>=0 && r.bottom<=innerHeight; }), "last diagnostic action is clipped");
      const active = page.getByRole("tabpanel", { name: "文字起こし", exact: true });
      await active.scrollIntoViewIfNeeded();
      check(await active.evaluate(e => e.clientHeight >= Math.min(480, innerHeight * .6) - 1), "expanded history collapses editor viewport");
      await section.scrollIntoViewIfNeeded();
      await page.screenshot({ path: `output/playwright/archive-long-${theme}-${width}-20261012.png` });
      passed++;
    }
    await page.getByRole("tab", { name: "議事録", exact: true }).click();
    await page.getByRole("tabpanel", { name: "議事録", exact: true }).scrollIntoViewIfNeeded();
    check(await page.locator(".archived-input-notices").evaluate(e => e.scrollHeight <= e.clientHeight + 1), "minutes tab restored nested scroll");
    await history.scrollIntoViewIfNeeded();
    await history.getByRole("button", { name: "次へ", exact: true }).click();
    check(await history.locator("li").count() === 2, "diagnostic pagination lost entries");
    await history.locator("summary").click();
    check(!(await history.evaluate(e => e.open)), "history cannot be collapsed");
    check(await page.locator(".meeting-detail-content").evaluate(e => e.scrollHeight <= e.clientHeight + 1), "collapsed history leaves unnecessary workspace scrolling");
    await page.setViewportSize({ width: 1280, height: 820 });
    await page.evaluate(() => {
      const f=window.closeFixture;
      f.archived.rows=[f.newArchivedRow({ title:"通知履歴の表示確認", count:0, loss:12 })];
      f.recordChanged({phase:"idle"});
      document.documentElement.dataset.theme="light";
    });
    await page.waitForFunction(() => document.querySelector(".archived-input-notices summary")?.textContent.includes("要確認 1件"));
    await history.locator("summary").click();
    await page.locator(".meeting-detail-content").evaluate(e => e.scrollTop = 0);
    await page.screenshot({ path:"output/playwright/archive-single-expanded-20261012.png" });
    check(await page.evaluate(() => window.closeFixture.calls.every(c => ![
      "start_recording", "stop_recording", "start_capture_test", "prepare_meeting_playback", "prepare_capture_test_playback",
    ].includes(c.command))), "layout verification requested capture/playback");
    return { passed: passed + 3, widths: [1280, 960, 360], themes: ["light", "dark"] };
  } finally { await page.context().unrouteAll(); }
}
