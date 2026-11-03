async (page, capturePath, { approveModelLicense, prepareClipboard }) => {
  const check = (ok, message) => { if (!ok) throw Error(message); }, results = [];
  await prepareClipboard(); await page.reload();
  const settings = () => page.getByRole("dialog", { name: "ローカルAI設定", exact: true });
  const review = () => page.getByRole("dialog", { name: "利用条件を確認", exact: true });
  const viewer = () => page.getByRole("dialog", { name: "モデルの利用条件", exact: true });
  const field = () => page.getByRole("combobox", { name: "文字起こしのモデル", exact: true });
  const row = () => page.locator(".model-selection-row").filter({ has: field() });
  const item = () => page.locator(".model-selection-item").filter({ has: field() });
  const count = command => page.evaluate(command => window.modelDownloadFixture.calls.filter(c => c.command === command).length, command);
  const mutationCount = () => page.evaluate(() => window.modelDownloadFixture.calls.filter(c => ["start_model_download", "place_downloaded_model", "check_model_compatibility", "preview_model_profile_application", "apply_model_profile_application", "cancel_model_inspection", "get_model_download_catalog"].includes(c.command)).length);
  const openSettings = async () => { await page.getByRole("button", { name: "ローカルAI設定", exact: true }).click(); await field().selectOption("official:synthetic-catalog"); };
  const openReview = () => row().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  const assertUntouched = async () => { check(await mutationCount() === 0, "review or cancellation started a model operation"); check(await page.evaluate(() => window.modelDownloadFixture.profiles[0].components[0]?.packageId) === "44444444-4444-4444-8444-444444444444", "review changed the active model"); };
  await openSettings();
  await item().getByRole("button", { name: "利用条件を見る", exact: true }).click();
  await viewer().locator(".model-license-names").waitFor();
  check(await viewer().getByRole("checkbox").count() === 0 && await viewer().getByRole("button", { name: "使用", exact: true }).count() === 0, "read-only terms contain an acknowledgment action");
  await viewer().locator("summary").click();
  const fullText = viewer().locator("pre");
  check((await fullText.innerText()).includes("<script>window.licenseExecuted = true</script>") && await page.evaluate(() => !window.licenseExecuted), "license text was executed or altered");
  check(await fullText.evaluate(e => getComputedStyle(e).whiteSpace === "pre-wrap" && e.scrollHeight > e.clientHeight), "long offline license is not readable in a bounded scroll area");
  const url = page.url(); await viewer().getByRole("button", { name: "出典をコピー", exact: true }).click();
  await viewer().getByText("出典をコピーしました。", { exact: true }).waitFor();
  check(page.url() === url && await viewer().locator("a[href]").count() === 0, "source action navigated externally");
  await page.keyboard.press("Escape"); await settings().waitFor(); await assertUntouched();
  check(await item().getByRole("button", { name: "利用条件を見る", exact: true }).evaluate(e => e === document.activeElement), "terms viewer did not restore its opener");
  results.push("read-only offline terms preserve escaped full text, copy source without navigation and return focus inside one settings modal");

  await field().selectOption("44444444-4444-4444-8444-444444444444");
  check((await item().locator(".model-license-summary").innerText()).includes("条件未確認"), "unmatched installed model guessed a permissive license");
  await item().getByRole("button", { name: "利用条件を見る", exact: true }).click();
  await viewer().locator(".model-license-names").getByText("モデル・実行エンジン：条件未確認", { exact: true }).waitFor();
  await page.keyboard.press("Escape"); await settings().waitFor(); await assertUntouched();
  await field().selectOption("official:synthetic-catalog");
  results.push("unmatched installed packages show unknown conditions and retain their active selection without guessing permissive rights");

  for (const dismiss of ["cancel", "escape", "close"]) {
    await openReview(); const checkbox = review().getByRole("checkbox"); await checkbox.waitFor();
    check(!await checkbox.isChecked() && await review().getByRole("button", { name: "ダウンロードして使用", exact: true }).isDisabled(), "fresh approval was prechecked or enabled");
    check(await page.locator("dialog[open]").count() === 1, "review opened a second native modal");
    if (dismiss === "cancel") {
      for (const theme of ["light", "dark"]) for (const width of [1200, 390]) {
        await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
        await page.setViewportSize({ width, height: 900 });
        check(await review().evaluate(e => e.getBoundingClientRect().width <= innerWidth && e.scrollWidth <= e.clientWidth + 1), "license review overflows a narrow viewport");
        await page.screenshot({ path: `model-license-review-${theme}-${width}.png` });
      }
      await page.setViewportSize({ width: 1200, height: 900 });
      await page.evaluate(() => { document.documentElement.dataset.theme = "light"; });
    }
    await checkbox.check();
    if (dismiss === "escape") await page.keyboard.press("Escape");
    else if (dismiss === "close") await review().getByRole("button", { name: "確認をキャンセル", exact: true }).click();
    else await review().getByRole("button", { name: "キャンセル", exact: true }).last().click();
    await settings().waitFor(); await assertUntouched();
  }
  results.push("cancel, close and Escape never start IO; every new action begins with an unchecked acknowledgement");

  await page.evaluate(() => { window.modelDownloadFixture.licenseDelay = true; });
  await openReview(); await review().getByText("利用条件を読み込んでいます…", { exact: true }).waitFor();
  await page.keyboard.press("Escape"); await settings().waitFor();
  await page.evaluate(() => { const f = window.modelDownloadFixture; f.licenseDelay = false; f.finishLicense(); });
  await assertUntouched(); check(await review().count() === 0, "late read-only response reopened a cancelled review");
  results.push("cancelled asynchronous disclosure reads cannot reopen or start a model pipeline");

  await openReview(); await review().getByRole("checkbox").check();
  await page.evaluate(() => { const f = window.modelDownloadFixture, recipe = JSON.parse(f.definition); f.originalDefinition = f.definition; recipe.manifest.engineVersion = "changed-runtime"; f.definition = JSON.stringify(recipe); });
  await review().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await review().getByRole("alert").waitFor(); await assertUntouched();
  check(!await review().getByRole("checkbox").isChecked(), "stale recipe retained acknowledgement");
  await page.keyboard.press("Escape"); await settings().waitFor();
  await page.evaluate(() => { window.modelDownloadFixture.definition = window.modelDownloadFixture.originalDefinition; });
  results.push("confirmation re-reads current native catalog and refuses a changed recipe before any operation token");

  await openReview(); await review().getByRole("checkbox").check();
  await page.evaluate(() => { window.modelDownloadFixture.policyRevision = 1; });
  await review().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await review().getByRole("alert").waitFor(); await assertUntouched();
  await page.keyboard.press("Escape"); await settings().waitFor();
  await page.evaluate(() => { window.modelDownloadFixture.policyRevision = 0; });
  results.push("changed disclosure bytes are refused while the selected model remains unchanged");

  for (const block of ["setLocked", "setRecordingActive", "setVisible"]) {
    await openReview(); await review().getByRole("checkbox").check();
    await page.evaluate(block => window.modelDownloadFixture[block](block !== "setVisible"), block);
    await review().waitFor({ state: "hidden" }); await assertUntouched();
    await page.evaluate(block => window.modelDownloadFixture[block](block === "setVisible"), block);
    await field().waitFor();
  }
  results.push("recording, locking or hiding settings invalidates approval without model mutation");

  await openReview(); await approveModelLicense();
  await row().getByRole("button", { name: "使用中", exact: true }).waitFor();
  for (const command of ["start_model_download", "place_downloaded_model", "check_model_compatibility", "preview_model_profile_application", "apply_model_profile_application"]) check(await count(command) === 1, `${command} did not run exactly once after approval`);
  check(await review().count() === 0 && await page.locator("dialog[open]").count() === 1, "confirmation did not return to its settings owner");
  await item().getByRole("button", { name: "利用条件を見る", exact: true }).click(); await viewer().getByRole("button", { name: "閉じる", exact: true }).first().click();
  check(await count("start_model_download") === 1 && await count("apply_model_profile_application") === 1, "viewing active model terms repeated work");
  results.push("one explicit approval runs download, placement, compatibility and application exactly once; active model terms stay read-only");
  await page.screenshot({ path: "model-license-settings.png" });
  return { passed: results.length, results };
}
