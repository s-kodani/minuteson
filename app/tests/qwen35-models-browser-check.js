async (page, capturePath, { approveModelLicense }) => {
  const check = (value, message) => { if (!value) throw Error(message); };
  const field = page.getByRole("combobox", { name: "議事録生成のモデル", exact: true });
  const row = page.locator(".model-selection-row").filter({ has: field });
  await field.waitFor();
  check(await page.getByRole("combobox").count() === 3, "model catalog did not load");
  // The select controls render before asynchronous catalog validation finishes.
  await page.waitForFunction(() => document.querySelectorAll('select[aria-label$="のモデル"] option[value^="official:"]').length === 12);
  const before = await field.locator("option").allTextContents();
  for (const size of ["4B", "9B"]) {
    const id = `official:llama-b11533-qwen35-${size.toLowerCase()}-q4km`;
    await field.selectOption(id);
    check(await field.locator("option:checked").textContent() === `Qwen 3.5 ${size} · Q4_K_M`, "Qwen3.5 label differs");
    check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 0, "selection started download");
  }
  for (const [size, hash] of [
    ["4B", "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4"],
    ["9B", "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8"],
  ]) {
    const id = `official:llama-b11533-qwen35-${size.toLowerCase()}-q4km`;
    await field.selectOption(id);
    const previous = await page.evaluate(() => window.modelDownloadFixture.profiles[0].profileVersion);
    await row.getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await approveModelLicense();
    await page.waitForFunction(previous => window.modelDownloadFixture.profiles[0].profileVersion === previous + 1, previous);
    await page.waitForFunction(() => document.querySelector('select[aria-label="議事録生成のモデル"]')?.closest(".model-selection-row")?.querySelector("button")?.disabled);
    const recipe = await page.evaluate(() => JSON.parse(window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").at(-1).args.definition));
    check(recipe.manifest.modelName === `Qwen3.5-${size}` && recipe.manifest.files.find(f => f.role === "llm").expectedHash === hash, "download used another model's bytes");
    check(await field.inputValue() === id, "model selection changed after applying");
    check(JSON.stringify(await field.locator("option").allTextContents()) === JSON.stringify(before), "installation duplicated a choice");
  }
  const inventory = page.getByRole("region", { name: "導入済みモデル管理", exact: true });
  for (const size of ["4B", "9B"]) check(await inventory.getByText(`Qwen 3.5 ${size} · Q4_K_M`, { exact: false }).count() === 1, "installed model missing or duplicated");
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 900 });
    check(await row.evaluate(el => el.getBoundingClientRect().right <= innerWidth), "model controls overflow");
  }
  await page.screenshot({ path: "output/playwright/qwen35-models.png" });
  return ["twelve catalog choices load; both Qwen3.5 sizes select without IO", "each Qwen3.5 download and global selection binds its own model bytes", "installed choices retain stable labels without duplicates at desktop and mobile widths"];
}
