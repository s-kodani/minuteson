async function (page, capturePath, { approveModelLicense }) {
  const check = (value, message) => { if (!value) throw Error(message); };
  const field = page.getByRole("combobox", { name: "議事録生成のモデル", exact: true });
  const row = page.locator(".model-selection-row").filter({ has: field });
  const models = [
    ["llama-b11960-gemma4-e4b-it-q4km", "Gemma 4 E4B IT · Q4_K_M"],
    ["llama-b11960-phi4-mini-instruct-q4km", "Phi-4-mini-instruct · Q4_K_M"],
    ["llama-b11960-lfm3-17b-q4km", "LFM3-1.7B · Q4_K_M"],
  ];
  await field.waitFor();
  await page.waitForFunction(() => document.querySelectorAll('select[aria-label="議事録生成のモデル"] option[value^="official:"]').length === 9);
  const before = await field.locator("option").allTextContents();
  for (const [id, label] of models) {
    await field.selectOption(`official:${id}`);
    check(await field.locator("option:checked").textContent() === label, "additional model label differs");
  }
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 0, "selection started IO");
  for (const [id, label] of models) {
    await field.selectOption(`official:${id}`);
    const previous = await page.evaluate(() => window.modelDownloadFixture.profiles[0].profileVersion);
    await row.getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await approveModelLicense();
    await page.waitForFunction(previous => window.modelDownloadFixture.profiles[0].profileVersion === previous + 1, previous);
    await page.waitForFunction(() => document.querySelector('select[aria-label="議事録生成のモデル"]')?.closest(".model-selection-row")?.querySelector("button")?.disabled);
    check(await field.inputValue() === `official:${id}`, "application lost the selected model");
    check(JSON.stringify(await field.locator("option").allTextContents()) === JSON.stringify(before), "installation duplicated or renamed choices");
    const inventory = page.getByRole("region", { name: "導入済みモデル管理", exact: true });
    check(await inventory.getByText(label, { exact: false }).count() === 1, "installed label missing or duplicated");
  }
  await page.evaluate(() => window.modelDownloadFixture.setVisible(false));
  await field.waitFor({ state: "hidden" });
  await page.evaluate(() => window.modelDownloadFixture.setVisible(true));
  await page.waitForFunction(() => document.querySelector('select[aria-label="議事録生成のモデル"]')?.value === "official:llama-b11960-lfm3-17b-q4km");
  await field.selectOption(`official:${models[0][0]}`);
  await row.getByRole("button", { name: "使用", exact: true }).click();
  await approveModelLicense();
  await page.waitForFunction(() => document.querySelector('select[aria-label="議事録生成のモデル"]')?.closest(".model-selection-row")?.querySelector("button")?.disabled);
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 3, "switching installed models downloaded again");
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 900 });
    check(await row.evaluate(el => el.getBoundingClientRect().right <= innerWidth), "model controls overflow");
  }
  await page.screenshot({ path: "output/playwright/additional-minutes-models.png" });
  return ["all nine minutes models load and new choices select without IO", "each new model downloads and applies with one stable label", "selection restores after remount and installed switching reuses bytes", "desktop and mobile model controls fit"];
}
