async function (page) {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const choices = [
    ["文字起こし", "whisper.cpp", ["whisper-synthetic-tiny", "whisper-synthetic-base", "whisper-synthetic-small", "whisper-synthetic-medium"]],
    ["話者分離", "sherpa-onnx", ["sherpa-1138-pyannote3-eres2net-base", "sherpa-1138-pyannote3-int8-eres2net-base"]],
    ["議事録生成", "llama.cpp", ["llama-b11533-qwen25-15b-q4km", "llama-b11533-qwen25-05b-q4km", "llama-b11533-qwen25-15b-q8"]],
  ];
  for (const [label, , ids] of choices) {
    const field = page.getByRole("combobox", { name: `${label}のモデル`, exact: true });
    await field.waitFor();
    for (const id of ids) {
      await field.selectOption(`official:${id}`);
      check(await field.inputValue() === `official:${id}`, "model choice did not change");
      check(await page.locator('.model-selection-row').filter({ has: field }).getByRole("button", { name: "ダウンロードして使用", exact: true }).isEnabled(), "choice has no download action");
    }
  }
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 0, "selection started IO");
  results.push("all nine model choices are selectable without automatic download or execution");
  const use = async (label, engine, id) => {
    const field = page.getByRole("combobox", { name: `${label}のモデル`, exact: true });
    const row = page.locator('.model-selection-row').filter({ has: field });
    await field.selectOption(`official:${id}`);
    const previous = await page.evaluate(() => window.modelDownloadFixture.profiles[0].profileVersion);
    await row.getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
    await page.waitForFunction(({ previous, engine }) => {
      const p = window.modelDownloadFixture.profiles[0];
      return p.profileVersion === previous + 1 && p.components.some(c => c.engine === engine);
    }, { previous, engine });
    await page.waitForFunction(label => {
      const s = document.querySelector(`select[aria-label="${label}のモデル"]`);
      return s?.closest('.model-selection-row')?.querySelector('button')?.disabled;
    }, label);
  };
  await use("文字起こし", "whisper.cpp", "whisper-synthetic-tiny");
  await use("議事録生成", "llama.cpp", "llama-b11533-qwen25-05b-q4km");
  await use("話者分離", "sherpa-onnx", "sherpa-1138-pyannote3-int8-eres2net-base");
  const initial = await page.evaluate(() => structuredClone(window.modelDownloadFixture.profiles[0]));
  check(initial.configurationComplete && initial.components.length === 3, "applying another engine lost an existing component");
  await use("議事録生成", "llama.cpp", "llama-b11533-qwen25-15b-q8");
  await use("話者分離", "sherpa-onnx", "sherpa-1138-pyannote3-eres2net-base");
  const latest = await page.evaluate(() => structuredClone(window.modelDownloadFixture.profiles[0]));
  check(latest.configurationComplete && latest.components.length === 3, "switching models lost global settings");
  check(latest.components.find(c => c.engine === "whisper.cpp").packageId === initial.components.find(c => c.engine === "whisper.cpp").packageId, "unrelated ASR choice changed");
  check(latest.components.find(c => c.engine === "llama.cpp").packageId !== initial.components.find(c => c.engine === "llama.cpp").packageId, "LLM choice was not switched");
  results.push("one-action setup and switching preserve one current global component for each engine");
  const recipes = await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").map(c => JSON.parse(c.args.definition)));
  check(recipes.length === 5, "model switch repeated a download");
  check(recipes[1].manifest.files.find(f => f.role === "llm").expectedHash === "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db", "0.5B bytes not bound");
  check(recipes[3].manifest.files.find(f => f.role === "llm").expectedHash === "d7efb072e7724d25048a4fda0a3e10b04bdef5d06b1403a1c93bd9f1240a63c8", "Q8 bytes not bound");
  check(recipes[2].manifest.files.find(f => f.role === "segmentation").expectedHash === "d582f4b4c6b48205de7e0643c57df0df5615a3c176189be3fc461e9d18827b5d", "int8 segmentation not bound");
  const inventory = page.getByRole("region", { name: "導入済みモデル", exact: true });
  for (const recipe of recipes) check(await inventory.getByText(recipe.manifest.modelName, { exact: true }).count() === 1, "installed earlier model was lost");
  check(await inventory.getByText("使用中", { exact: true }).count() === 3, "in-use count differs from the three selected engines");
  results.push("selected model bytes bind each acquisition and previous installed models stay in the inline inventory");
  await page.evaluate(() => window.modelDownloadFixture.setVisible(false));
  await page.getByRole("combobox").first().waitFor({ state: "hidden" });
  await page.evaluate(() => window.modelDownloadFixture.setVisible(true));
  await page.getByRole("region", { name: "導入済みモデル", exact: true }).waitFor();
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 5, "remount downloaded again");
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const metrics = await page.locator('.model-selection-row').evaluateAll(rows => rows.map(row => {
      const s = row.querySelector('select').getBoundingClientRect(), b = row.querySelector('button').getBoundingClientRect();
      return { s: s.height, b: b.height, right: Math.max(s.right, b.right) };
    }));
    check(metrics.length === 3 && metrics.every(m => m.s === 40 && m.b === 40 && m.right <= width), "controls differ or overflow");
  }
  results.push("reopening reads current models without repeating work; selectors and buttons retain matching heights at both widths");
  await page.screenshot({ path: "all-model-choices.png" });
  return { passed: results.length, results };
}
