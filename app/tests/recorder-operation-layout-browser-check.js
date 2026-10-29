// Production layout with synthetic notices only; never start capture or playback.
async page => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,
      '<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  try {
    await page.setViewportSize({ width:1280, height:820 });
    await page.goto("http://127.0.0.1:1420/");
    await page.waitForFunction(() => !!window.closeFixture);
    const title = await page.evaluate(() => window.dataFixture.meetings[0].title);
    await page.getByRole("button", { name:title, exact:true }).click();
    const host = page.locator(".recorder-operation-notifications");
    check(await host.evaluate(e => e.getBoundingClientRect().height === 0), "empty notices reserve a blank row");
    await page.evaluate(async () => {
      const { dataSaveIPC } = await import("/tests/data-save-browser-fixture.js");
      await dataSaveIPC("publish_recorder_operation_notice", { source:"window_action", expectedRevision:0, message:"合成通知：録音パネルの操作を確認してください" });
    });
    await host.getByText("要対応：録音パネルの操作 1件", { exact:true }).waitFor();
    let passed = 1;
    for (const theme of ["light", "dark"]) for (const width of [1280, 960, 360]) {
      await page.setViewportSize({ width, height:820 });
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      check(await host.evaluate(e => {
        const status = e.querySelector(".issue-status"), label=status.querySelector("strong");
        const header=e.parentElement.querySelector(".topbar");
        const pad=parseFloat(getComputedStyle(header).paddingLeft), rect=label.getBoundingClientRect();
        return Math.abs(rect.left - (header.getBoundingClientRect().left + pad)) < 1
          && parseFloat(getComputedStyle(status).paddingRight) === pad
          && rect.right <= e.getBoundingClientRect().right - pad + 1
          && status.scrollHeight <= status.clientHeight + 1;
      }), `notice gutter/overflow mismatch: ${theme}/${width}`);
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "notice causes horizontal overflow");
      await page.screenshot({ path:`output/playwright/recorder-notice-gutter-${theme}-${width}-20261012.png` });
      passed++;
    }
    await page.evaluate(() => { window.dataFixture.operationReadFailure = true; });
    await host.getByText("要対応：録音パネルの操作通知", { exact:true }).waitFor();
    check(await host.locator(".issue-status").evaluateAll(rows => rows.length === 2
      && rows.every(e => parseFloat(getComputedStyle(e).paddingLeft) === 16 && e.scrollHeight <= e.clientHeight + 1)), "read failures lose the shared gutter");
    passed++;
    await page.evaluate(async () => {
      window.dataFixture.operationReadFailure = false;
      const { dataSaveIPC } = await import("/tests/data-save-browser-fixture.js");
      await dataSaveIPC("publish_recorder_operation_notice", { source:"window_action", expectedRevision:1, message:null });
    });
    await page.waitForFunction(() => document.querySelector(".recorder-operation-notifications").children.length === 0);
    check(await host.evaluate(e => e.getBoundingClientRect().height === 0), "resolved notices leave a blank row");
    check(await page.evaluate(() => window.closeFixture.calls.every(c => !["start_recording", "stop_recording", "start_capture_test", "prepare_meeting_playback", "prepare_capture_test_playback"].includes(c.command))), "layout verification requested capture/playback");
    return { passed:passed+1, widths:[1280,960,360], themes:["light","dark"] };
  } finally { await page.context().unrouteAll(); }
}
