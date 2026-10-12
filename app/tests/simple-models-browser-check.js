async function (page) {
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
  await page.getByRole("button", { name: "現在の状態を確認", exact: true }).waitFor();
  const count = command => page.evaluate(command => window.modelDownloadFixture.calls.filter(call => call.command === command).length, command);
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 0, "uncertain download continued writing");
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; });
  await page.getByRole("button", { name: "現在の状態を確認", exact: true }).click();
  await page.getByRole("button", { name: "現在の状態を確認", exact: true }).waitFor({ state: "hidden" });
  await asrRow.getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await page.waitForFunction(() => window.modelDownloadFixture.profiles[0].components.length === 1);
  await select.waitFor({state:"visible"});
  await page.waitForFunction(() => document.querySelector('.model-selection-row button')?.disabled);
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 1 && await count("check_model_compatibility") === 1 && await count("apply_model_profile_application") === 1, "one click did not verify and apply the existing download");
  results.push("download response loss blocks writers; verified readback reuses one download and completes setup in one action");
  const inventory = page.getByRole("region", { name: "導入済みモデル", exact: true });
  check(await inventory.getByText("使用中", {exact:true}).count() === 1, "selected model missing from inline inventory");
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
  await page.screenshot({path:"simple-models.png"});
  return {passed:results.length,results};
}
