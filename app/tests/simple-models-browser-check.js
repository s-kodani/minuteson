(async function (page, capturePath, { approveModelLicense }) {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  await page.evaluate(() => window.modelDownloadFixture.setParent(true));
  const select = page.getByRole("combobox", { name: "文字起こしのモデル", exact: true });
  const asrRow = page.locator('.model-selection-row').filter({has:select});
  await select.waitFor();
  for (const theme of ["light", "dark"]) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
    for (const width of [1200, 800, 390]) {
      await page.setViewportSize({width, height:900});
      const metrics = await page.locator('.model-selection-row').evaluateAll(rows => rows.map(row => {
        const select = row.querySelector('select'), button = row.querySelector('button');
        const field = select.getBoundingClientRect(), action = button.getBoundingClientRect();
        const fieldStyle = getComputedStyle(select), actionStyle = getComputedStyle(button);
        return {height:field.height, buttonHeight:action.height, top:field.top, buttonTop:action.top,
          font:fieldStyle.fontSize, buttonFont:actionStyle.fontSize,
          lineHeight:fieldStyle.lineHeight, buttonLineHeight:actionStyle.lineHeight,
          right:Math.max(field.right,action.right)};
      }));
      check(metrics.length === 3, "expected all three model selectors");
      for (const metric of metrics) {
        check(metric.height === 40 && Math.abs(metric.height - metric.buttonHeight) < 1, `model control heights differ at ${width}px (${theme})`);
        check(metric.font === metric.buttonFont && metric.lineHeight === metric.buttonLineHeight, "model control fonts differ");
        if (width > 600) check(Math.abs(metric.top - metric.buttonTop) < 1, "model controls are vertically misaligned");
        check(metric.right <= width, "model controls overflow the viewport");
      }
    }
  }
  results.push("all three model selectors match button height, typography and alignment in both themes at three viewport widths");
  await page.setViewportSize({width:1200,height:900});
  await select.selectOption("official:synthetic-catalog");
  check(await page.getByRole("combobox").count() === 3, "expected three simple model choices");
  check(await page.getByRole("button", { name: /取得定義|取得記録|配置・構成記録/ }).count() === 0, "obsolete setup actions remain");
  await page.evaluate(() => { window.modelDownloadFixture.mode = "lost"; });
  await asrRow.getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await approveModelLicense();
  const count = command => page.evaluate(command => window.modelDownloadFixture.calls.filter(call => call.command === command).length, command);
  const readyToRetry = () => page.waitForFunction(() => window.modelDownloadFixture.calls.some(call => call.command === "get_model_download_lifetime") && !document.querySelector('.model-inspection>p[role="status"]') && !document.querySelector('[aria-label="文字起こしのモデル"]')?.disabled);
  const recovering = page.locator('.model-inspection>p[role="status"]');
  await recovering.waitFor();
  check(await page.getByRole("button", { name: "現在の状態を確認", exact: true }).count() === 0, "internal status check requires a user action");
  await readyToRetry();
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 0, "uncertain download continued writing");
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; });
  await asrRow.getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await approveModelLicense();
  await page.waitForFunction(() => window.modelDownloadFixture.profiles[0].components.length === 1);
  await select.waitFor({state:"visible"});
  await page.waitForFunction(() => document.querySelector('.model-selection-row button')?.disabled);
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 1 && await count("check_model_compatibility") === 1 && await count("apply_model_profile_application") === 1, "one click did not verify and apply the existing download");
  results.push("download response loss blocks writers; verified readback reuses one download and completes setup in one action");
  const inventory = page.getByRole("region", { name: "導入済みモデル管理", exact: true });
  check(await inventory.getByText(/ · 使用中$/).count() === 1, "selected model missing from inline inventory");
  results.push("three compact choices and inline installed models replace setup records and review controls");
  await page.evaluate(() => { window.modelDownloadFixture.setVisible(false); });
  await select.waitFor({state:"hidden"}); await page.evaluate(() => window.modelDownloadFixture.setVisible(true));
  await select.waitFor();
  await page.waitForFunction(() => document.querySelector('.model-selection-row button')?.disabled);
  check(await count("start_model_download") === 1 && await count("apply_model_profile_application") === 1, "remount repeated model work");
  results.push("reopening settings reads the current selection without downloading or applying again");
  check(await page.getByRole("button", {name:"ローカルから取り込む",exact:true}).count() === 0, "local import action remains");
  check(await select.locator('option[value="44444444-4444-4444-8444-444444444444"]').evaluate(e => e.disabled), "unverified local executable can be selected");
  check(await count("inspect_model_package") === 0 && await count("place_model_package") === 0 && await count("select_model_files") === 0, "local import IPC was called");
  results.push("local import is removed; existing unverified runtimes cannot be executed silently");
  // Each page load resets only this synthetic application's in-memory state.
  // No network/model/OS permission work is performed by the fixture.
  const start = async () => { await asrRow.getByRole("button", { name: /^(ダウンロードして使用|使用)$/ }).click(); await approveModelLicense(); };
  for (const mode of ["modelFileFailure", "downloadFailure", "beforeIntentFailure", "placeFailure", "checkFailure", "previewFailure", "applyFailure"]) {
    await page.reload(); await select.waitFor(); await select.selectOption("official:synthetic-catalog");
    await page.evaluate(mode => { window.modelDownloadFixture.mode = mode; }, mode);
    await start(); await readyToRetry();
    if (mode === "modelFileFailure") {
      const notice = page.locator('.toast-card').getByRole("alert").filter({hasText:"モデルを読み込めません。"});
      await notice.waitFor();
      const message = await notice.innerText();
      check(message.includes("もう一度お試しください。"), "model failure does not offer a retry");
      check(!/親フォルダー|リンク|Error:|現在の状態を確認/.test(message), "model notification exposes internal diagnostics or requires a status check");
      check(await notice.evaluate(element => element.scrollHeight <= element.clientHeight + 1), "model recovery guidance is truncated");
      await page.waitForFunction(() => { const toast = document.querySelector('.toast-card')?.closest('[data-sonner-toast]'); return toast && getComputedStyle(toast).opacity === "1"; });
      await page.screenshot({path:"model-file-failure-notice.png"});
    }
    const before = await count("start_model_download");
    check(await count("place_downloaded_model") <= 1 && await count("apply_model_profile_application") <= 1, "automatic recovery repeated a write");
    await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; });
    check(await count("start_model_download") === before && await count("apply_model_profile_application") <= 1, "status recovery performed new work");
    await start();
    await page.waitForFunction(() => window.modelDownloadFixture.profiles[0].components.length === 1);
    await page.waitForFunction(() => document.querySelector('.model-selection-row button')?.disabled);
    check(await count("start_model_download") === before + (["modelFileFailure", "downloadFailure", "beforeIntentFailure"].includes(mode) ? 1 : 0), `${mode}: completed download was repeated or explicit retry was blocked`);
    if (mode === "downloadFailure") check(await page.evaluate(() => [...window.modelDownloadFixture.operations.values()].some(item => item.state === "incomplete")), "incomplete bytes were silently adopted/deleted");
    results.push(`${mode}: confirmed terminal state releases the selector; explicit action retries only unfinished setup`);
  }
  for (const fault of ["failCancel", "failLifetime", "failCatalog", "readMissing", "forgedReadback"]) {
    await page.reload(); await select.waitFor(); await select.selectOption("official:synthetic-catalog");
    await page.evaluate(() => { window.modelDownloadFixture.mode = "lost"; });
    await start();
    await page.evaluate(fault => { window.modelDownloadFixture[fault] = true; }, fault);
    await recovering.waitFor();
    await page.locator('.toast-card').getByRole("alert").filter({hasText:"アプリを再起動してください。"}).waitFor();
    check(await recovering.isVisible() && await select.isDisabled(), `${fault}: uncertain result released model selection`);
    check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 0, `${fault}: result checking replayed work`);
    await page.evaluate(fault => { window.modelDownloadFixture[fault] = false; window.modelDownloadFixture.mode = "success"; }, fault);
    await readyToRetry();
    await start(); await page.waitForFunction(() => window.modelDownloadFixture.profiles[0].components.length === 1);
    check(await count("start_model_download") === 1, `${fault}: recovery discarded a completed download`);
    results.push(`${fault}: unknown state remains fenced until valid readback, then reuses the completed download`);
  }
  await page.reload(); await select.waitFor(); await select.selectOption("official:synthetic-catalog");
  await page.evaluate(() => { window.modelDownloadFixture.mode = "earlyLost"; });
  await start(); await recovering.waitFor();
  await page.waitForFunction(() => window.modelDownloadFixture.calls.filter(call => call.command === "get_model_download_lifetime").length >= 2);
  check(await recovering.isVisible() && await select.isDisabled() && await count("read_model_download") === 0, "active download was treated as terminal");
  await page.evaluate(() => { try { window.modelDownloadFixture.finish(); } catch {} window.modelDownloadFixture.mode = "success"; });
  await readyToRetry();
  check(await count("start_model_download") === 1, "stop/result checking automatically retried an incomplete download");
  await start(); await page.waitForFunction(() => window.modelDownloadFixture.profiles[0].components.length === 1);
  check(await count("start_model_download") === 2, "explicit retry after confirmed stop did not finish");
  results.push("active writer blocks readback; terminal cancellation allows only a new explicit action");

  await page.reload(); await select.waitFor(); await select.selectOption("official:synthetic-catalog");
  await page.evaluate(() => { window.modelDownloadFixture.mode = "lost"; window.modelDownloadFixture.failCancel = true; });
  await start(); await recovering.waitFor();
  await page.evaluate(() => window.modelDownloadFixture.setVisible(false));
  await select.waitFor({state:"hidden"});
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; window.modelDownloadFixture.failCancel = false; window.modelDownloadFixture.setVisible(true); });
  await select.waitFor(); await readyToRetry();
  await start(); await page.waitForFunction(() => window.modelDownloadFixture.profiles[0].components.length === 1);
  check(await count("start_model_download") === 1 && await count("apply_model_profile_application") === 1, "remount duplicated or abandoned the pending download");
  results.push("reopening settings automatically reconciles an uncertain operation without duplicating its download");

  await page.reload(); await select.waitFor(); await select.selectOption("official:synthetic-catalog");
  await page.evaluate(async () => { const f = window.modelDownloadFixture, id = await f.seed(); f.operations.get(id).state = "incomplete"; });
  await start(); await page.waitForFunction(() => window.modelDownloadFixture.profiles[0].components.length === 1);
  check(await count("start_model_download") === 1 && await count("cancel_model_inspection") === 1 && await count("read_model_download") === 1, "persisted incomplete staging blocked explicit retry or escaped verification");
  results.push("previous incomplete staging is fenced and verified, not adopted, before a fresh explicit download");

  await page.reload(); await select.waitFor(); await select.selectOption("official:synthetic-catalog");
  await page.evaluate(() => { window.modelDownloadFixture.mode = "applyLost"; });
  await start();
  // Successful readback can finish before Playwright observes the recovery notice.
  await readyToRetry();
  await asrRow.getByRole("button", { name: "使用中", exact: true }).waitFor();
  check(await asrRow.getByRole("button").isDisabled() && await count("apply_model_profile_application") === 1, "lost successful application was replayed or left actionable as unapplied");
  results.push("successful selection with a lost reply is read back as selected without reapplying");
  await page.screenshot({path:"simple-models.png"});
  return {passed:results.length,results};
})
