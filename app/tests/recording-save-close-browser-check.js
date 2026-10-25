async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const title = "録音の保存完了後の終了を確認";
  const modal = () => page.getByRole("dialog", { name: title, exact: true });
  const accept = () => modal().getByRole("button", { name: "保存完了を待って終了へ進む", exact: true });
  const calls = command => page.evaluate(command => window.closeFixture.calls.filter(call => call.command === command).length, command);
  const boot = async phase => {
    await page.goto("http://127.0.0.1:1420/");
    await page.waitForFunction(() => !!window.closeFixture && !!document.querySelector(".meeting-table tbody tr"));
    await page.evaluate(phase => window.closeFixture.recordChanged({ phase, sessionId: crypto.randomUUID(), meetingId: window.dataFixture.meetings[0].id }), phase);
  };
  const saved = () => page.evaluate(() => window.closeFixture.recordChanged({ phase: "idle", sessionId: null, meetingId: null }));
  try {
    for (const phase of ["stopping", "saving"]) {
      await boot(phase);
      await page.getByRole("button", { name: /^ジョブ一覧/ }).click();
      const jobs = page.getByRole("region", { name: "バックグラウンドのジョブ一覧" });
      await jobs.getByText("録音の保存", { exact: true }).waitFor();
      check(await jobs.locator("progress").getAttribute("value") === null, "save job fabricated a percentage");
      if (phase === "saving") await page.screenshot({ path: "output/windows-qa-work/recording-save-job.png" });
      await page.evaluate(() => window.closeFixture.request(false)); // Main window X, not explicit app quit.
      await modal().waitFor();
      check((await modal().innerText()).includes("既存会議"), "save warning did not identify the meeting by title");
      if (phase === "saving") await page.screenshot({ path: "output/windows-qa-work/recording-save-close-warning.png" });
      await page.keyboard.press("Escape");
      await modal().waitFor({ state: "hidden" });
      await saved();
      await jobs.getByText("録音の保存", { exact: true }).waitFor({ state: "hidden" });
      check(await calls("finish_main_close") === 0 && await calls("stop_recording") === 0, "declining X hid or exited the app, or resent stop");
      results.push(`${phase}: visible job, X warns, declining keeps the app after later save completion`);
    }
    for (const quit of [false, true]) {
      await boot("saving");
      await page.evaluate(quit => window.closeFixture.request(quit), quit);
      await accept().dblclick();
      await modal().getByRole("status").waitFor();
      await page.keyboard.press("Escape");
      check(await modal().isVisible() && await calls("finish_main_close") === 0, "app hid or exited before actual save");
      await saved();
      await page.waitForFunction(() => window.closeFixture.calls.some(call => call.command === "finish_main_close"));
      check(await calls("finish_main_close") === 1 && await calls("stop_recording") === 0, "close or stop was repeated");
      results.push(`${quit ? "quit" : "X"}: waits for actual idle and closes once without repeating stop`);
    }
    for (const failure of ["save", "read", "owner"]) {
      await boot("saving");
      await page.evaluate(() => window.closeFixture.request(false));
      await accept().click();
      await modal().getByRole("status").waitFor();
      await page.evaluate(failure => {
        if (failure === "read") window.closeFixture.failRecordingSnapshot = true;
        else return window.closeFixture.recordChanged(failure === "save" ? { phase: "error", errorMessage: "保存失敗" } : { sessionId: crypto.randomUUID() });
      }, failure);
      await modal().getByRole("alert").waitFor();
      check(await calls("finish_main_close") === 0 && await calls("stop_recording") === 0, "unverified save exited or resent stop");
      await modal().getByRole("button", { name: "確認を閉じる", exact: true }).click();
      results.push(`${failure} failure keeps app open without closing or stopping again`);
    }
    await boot("saving");
    await page.evaluate(() => window.closeFixture.request(false));
    await accept().click();
    await modal().getByRole("status").waitFor();
    await page.evaluate(() => { window.closeFixture.activity.analysisBusy = true; });
    await saved();
    const analysis = page.getByRole("dialog", { name: "進行中の処理を停止して終了しますか？", exact: true });
    await analysis.waitFor();
    check(await calls("finish_main_close") === 0, "post-save analysis quit without review");
    await analysis.getByRole("button", { name: "安全な停止を待って終了する", exact: true }).click();
    await page.waitForFunction(() => window.closeFixture.calls.some(call => call.command === "finish_main_close"));
    results.push("analysis admitted after save retains its normal shutdown warning");
    return { passed: results.length, results };
  } finally {
    await page.goto("about:blank");
  }
}
