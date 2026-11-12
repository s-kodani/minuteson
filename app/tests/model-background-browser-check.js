// Production workspace and close coordination; synthetic model IPC, no network IO.
async (page, capturePath, { approveModelLicense }) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  const models = () => page.getByRole("region", { name: "AIモデル", exact: true });
  const row = () => models().locator(".model-selection-row").filter({ has: page.getByRole("combobox", { name: "文字起こしのモデル", exact: true }) });
  const background = () => page.getByRole("dialog", { name: "バックグラウンドで継続しますか？", exact: true });
  const requestClose = () => page.evaluate(() => window.operationProgressFixture.requestClose(false));
  const assertContinues = async () => {
    check(await page.evaluate(() => {
      const f = window.modelDownloadFixture;
      return f.active.size === 1 && f.cancelled.size === 0 && !f.calls.some(call => call.command === "cancel_model_inspection");
    }), "close confirmation cancelled active model preparation");
  };
  for (const platform of ["macos", "windows"]) {
    await page.goto(`http://127.0.0.1:1420/tests/operation-progress-browser.html?resident=${platform}`);
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await models().getByRole("combobox", { name: "文字起こしのモデル", exact: true }).selectOption("official:synthetic-catalog");
    await page.evaluate(() => { window.modelDownloadFixture.mode = "delay"; });
    await row().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
    await approveModelLicense();
    await page.waitForFunction(() => window.modelDownloadFixture.active.size === 1);

    await requestClose();
    await background().waitFor();
    await assertContinues();
    await background().getByRole("button", { name: "確認をキャンセル", exact: true }).click();
    await background().waitFor({ state: "hidden" });
    await assertContinues();
    check(!await page.evaluate(() => window.operationProgressFixture.hidden), "declining continuation hid the window");

    await requestClose();
    await page.evaluate(() => { window.operationProgressFixture.hideFailure = true; });
    await background().getByRole("button", { name: "継続して画面を閉じる", exact: true }).click();
    await page.getByRole("alert").filter({ hasText: "模擬ウィンドウ非表示失敗" }).waitFor();
    await assertContinues();
    check(!await page.evaluate(() => window.operationProgressFixture.hidden), "failed hide falsely closed the window");
    await page.keyboard.press("Escape");
    await background().waitFor({ state: "hidden" });
    await page.evaluate(() => { window.operationProgressFixture.hideFailure = false; });

    await requestClose();
    await background().getByRole("button", { name: "継続して画面を閉じる", exact: true }).click();
    await page.waitForFunction(() => window.operationProgressFixture.hidden);
    await assertContinues();
    await page.evaluate(() => window.modelDownloadFixture.finish());
    await page.waitForFunction(() => window.modelDownloadFixture.calls.some(call => call.command === "place_downloaded_model"));
    await assertContinues();
    await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; window.modelDownloadFixture.finish(); });
    await row().getByRole("button", { name: "使用中", exact: true }).waitFor();
    await page.getByRole("region", { name: "進行中の処理", exact: true }).waitFor({ state: "hidden" });
    check(await page.evaluate(() => {
      const m = window.modelDownloadFixture, f = window.operationProgressFixture;
      return f.closeCalls.length === 2 && m.calls.filter(call => call.command === "start_model_download").length === 1
        && m.calls.filter(call => call.command === "apply_model_profile_application").length === 1
        && !m.calls.some(call => call.command === "cancel_model_inspection")
        && !window.dataFixture.calls.some(call => call.command === "finish_main_close");
    }), "background continuation stopped, restarted or duplicated model preparation");
    results.push(`${platform}: declining, failing or accepting window close preserves download; background placement and application complete once`);
  }
  return { passed: results.length, results };
}
