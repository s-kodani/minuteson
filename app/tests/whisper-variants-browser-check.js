async function (page) {
  const check = (value, message) => { if (!value) throw Error(message); }, results = [];
  const field = page.getByRole("combobox", { name: "文字起こしのモデル", exact: true });
  await field.waitFor();
  const row = page.locator('.model-selection-row').filter({ has: field });
  const names = ["tiny", "base", "small", "medium"];
  for (const name of names) {
    await field.selectOption(`official:whisper-synthetic-${name}`);
    check(await field.inputValue() === `official:whisper-synthetic-${name}`, "model choice did not change");
    check(await row.getByRole("button", { name: "ダウンロードして使用", exact: true }).isEnabled(), "choice has no download action");
  }
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 0,
    "selecting a choice downloaded a model");
  results.push("all four named multilingual choices are selectable without starting download or CPU execution");
  for (const name of ["tiny", "base"]) {
    await field.selectOption(`official:whisper-synthetic-${name}`);
    await row.getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
    await page.waitForFunction(name => window.modelDownloadFixture.profiles[0].components[0]?.modelName === `Whisper ${name}（多言語）`, name);
    await row.getByRole("button", { name: "使用", exact: true }).waitFor();
    check(await row.getByRole("button", { name: "使用", exact: true }).isDisabled(), "applied model can be applied twice");
  }
  const calls = await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").map(c => JSON.parse(c.args.definition)));
  check(calls.length === 2, "switching models repeated acquisition");
  check(calls[0].manifest.files[3].relativePath === "models/ggml-tiny.bin" && calls[1].manifest.files[3].relativePath === "models/ggml-base.bin", "download did not bind the selected model");
  const inventory = page.getByRole("region", { name: "導入済みモデル", exact: true });
  check(await inventory.getByText("Whisper tiny（多言語）", { exact: true }).count() === 1, "previous installed model was lost");
  check(await inventory.getByText("Whisper base（多言語）", { exact: true }).count() === 1, "new installed model missing");
  check(await inventory.getByText("使用中", { exact: true }).count() === 1, "multiple models marked in use");
  results.push("one action downloads and applies the chosen model; switching retains the earlier installed model");
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const metrics = await row.evaluate(e => {
      const s = e.querySelector('select').getBoundingClientRect(), b = e.querySelector('button').getBoundingClientRect();
      return { s: s.height, b: b.height, right: Math.max(s.right, b.right) };
    });
    check(metrics.s === 40 && metrics.b === 40 && metrics.right <= width, "variant selector has mismatched height or overflows");
  }
  results.push("model selectors retain matching button height and fit wide and narrow viewports");
  await page.screenshot({ path: "whisper-variants.png" });
  return { passed: results.length, results };
}
