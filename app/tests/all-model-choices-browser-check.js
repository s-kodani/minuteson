(async function (page, capturePath, { approveModelLicense }) {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const choices = [
    ["文字起こし", ["whisper-synthetic-medium", "whisper-synthetic-large-v3", "whisper-synthetic-large-v3-turbo", "reazonspeech-k2-v2-int8-fp32", "reazonspeech-k2-v2-fp32"]],
    ["話者分離", ["sherpa-1138-pyannote3-eres2net-base", "sherpa-1138-pyannote3-int8-eres2net-base"]],
    ["議事録生成", ["llama-b11533-qwen3-8b-q4km", "llama-b11533-qwen3-4b-q4km", "llama-b11533-qwen35-4b-q4km", "llama-b11533-qwen35-9b-q4km", "llama-b11960-gemma4-e4b-it-q4km"]],
  ];
  for (const [label, ids] of choices) {
    const field = page.getByRole("combobox", { name: `${label}のモデル`, exact: true });
    await field.locator(`option[value="official:${ids[0]}"]`).waitFor({ state: "attached" });
    const offered = await field.locator('option[value^="official:"]').evaluateAll(options => options.map(option => option.value));
    check(JSON.stringify(offered) === JSON.stringify(ids.map(id => `official:${id}`)), `unexpected official choices for ${label}`);
    for (const id of ids) {
      await field.selectOption(`official:${id}`);
      check(await field.inputValue() === `official:${id}`, "model choice did not change");
      check(await page.locator('.model-selection-row').filter({ has: field }).getByRole("button", { name: "ダウンロードして使用", exact: true }).isEnabled(), "choice has no download action");
    }
  }
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 0, "selection started IO");
  results.push("all twelve model choices are selectable without automatic download or execution");
  const use = async (label, engine, id) => {
    const field = page.getByRole("combobox", { name: `${label}のモデル`, exact: true });
    const row = page.locator('.model-selection-row').filter({ has: field });
    await field.selectOption(`official:${id}`);
    const beforeOptions = await field.locator('option').allTextContents();
    const labelText = await field.locator(`option[value="official:${id}"]`).textContent();
    const previous = await page.evaluate(() => window.modelDownloadFixture.profiles[0].profileVersion);
    await row.getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
    await approveModelLicense();
    await page.waitForFunction(({ previous, engine }) => {
      const p = window.modelDownloadFixture.profiles[0];
      return p.profileVersion === previous + 1 && p.components.some(c => c.engine === engine);
    }, { previous, engine });
    await page.waitForFunction(label => {
      const s = document.querySelector(`select[aria-label="${label}のモデル"]`);
      return s?.closest('.model-selection-row')?.querySelector('button')?.disabled;
    }, label);
    check(await field.inputValue() === `official:${id}`, "download changed the selected model identity");
    check(JSON.stringify(await field.locator('option').allTextContents()) === JSON.stringify(beforeOptions), "download duplicated or renamed a choice");
    check(await page.getByRole("region", {name:"導入済みモデル管理"}).getByText(labelText, {exact:true}).count() === 1, "installed model label differs from its option");
  };
  await use("文字起こし", "whisper.cpp", "whisper-synthetic-medium");
  await use("議事録生成", "llama.cpp", "llama-b11533-qwen3-4b-q4km");
  await use("話者分離", "sherpa-onnx", "sherpa-1138-pyannote3-int8-eres2net-base");
  const initial = await page.evaluate(() => structuredClone(window.modelDownloadFixture.profiles[0]));
  check(initial.configurationComplete && initial.components.length === 3, "applying another engine lost an existing component");
  await use("議事録生成", "llama.cpp", "llama-b11960-gemma4-e4b-it-q4km");
  await use("話者分離", "sherpa-onnx", "sherpa-1138-pyannote3-eres2net-base");
  const latest = await page.evaluate(() => structuredClone(window.modelDownloadFixture.profiles[0]));
  check(latest.configurationComplete && latest.components.length === 3, "switching models lost global settings");
  check(latest.components.find(c => c.engine === "whisper.cpp").packageId === initial.components.find(c => c.engine === "whisper.cpp").packageId, "unrelated ASR choice changed");
  check(latest.components.find(c => c.engine === "llama.cpp").packageId !== initial.components.find(c => c.engine === "llama.cpp").packageId, "LLM choice was not switched");
  await use("議事録生成", "llama.cpp", "llama-b11533-qwen35-4b-q4km");
  await use("議事録生成", "llama.cpp", "llama-b11533-qwen3-8b-q4km");
  results.push("one-action setup and switching preserve one current global component for each engine");
  const recipes = await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").map(c => JSON.parse(c.args.definition)));
  check(recipes.length === 7, "model switch repeated a download");
  check(recipes[1].manifest.files.find(f => f.role === "llm").expectedHash === "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5", "Qwen3 4B bytes not bound");
  check(recipes[3].manifest.files.find(f => f.role === "llm").expectedHash === "85a896a047553e842f25297ee5b031d64ff30147d9c4af17b1e4b394cd1fab87", "Gemma 4 bytes not bound");
  check(recipes[2].manifest.files.find(f => f.role === "segmentation").expectedHash === "d582f4b4c6b48205de7e0643c57df0df5615a3c176189be3fc461e9d18827b5d", "int8 segmentation not bound");
  check(recipes[5].manifest.files.find(f => f.role === "llm").expectedHash === "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4", "Qwen3.5 4B bytes not bound");
  const inventory = page.getByRole("region", { name: "導入済みモデル管理", exact: true });
  check(await inventory.locator('li').count() === recipes.length + 1, "installed earlier model was lost");
  check(await inventory.getByText("Gemma 4 E4B IT · Q4_K_M", {exact:true}).count() === 1, "Gemma label changed after installation");
  check(await inventory.getByText("Qwen 3 4B · Q4_K_M", {exact:true}).count() === 1, "Qwen3 4B was not added");
  check(await inventory.getByText("Qwen 3 8B · Q4_K_M", {exact:true}).count() === 1, "Qwen3 was not added");
  check(await inventory.getByText(/ · 使用中$/u).count() === 3, "in-use count differs from the three selected engines");
  results.push("selected model bytes bind each acquisition and previous installed models stay in the inline inventory");
  await page.evaluate(() => window.modelDownloadFixture.setVisible(false));
  await page.getByRole("combobox").first().waitFor({ state: "hidden" });
  await page.evaluate(() => window.modelDownloadFixture.setVisible(true));
  await page.getByRole("region", { name: "導入済みモデル管理", exact: true }).waitFor();
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 7, "remount downloaded again");
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const metrics = await page.locator('.model-selection-list .model-selection-row').evaluateAll(rows => rows.map(row => {
      const s = row.querySelector('select').getBoundingClientRect(), b = row.querySelector('button').getBoundingClientRect();
      return { s: s.height, b: b.height, right: Math.max(s.right, b.right) };
    }));
    check(metrics.length === 3 && metrics.every(m => m.s === 40 && m.b === 40 && m.right <= width), "controls differ or overflow");
  }
  results.push("reopening reads current models without repeating work; selectors and buttons retain matching heights at both widths");
  const llmField = page.getByRole("combobox", {name:"議事録生成のモデル"});
  await page.waitForFunction(() => document.querySelector('select[aria-label="議事録生成のモデル"]')?.value === "official:llama-b11533-qwen3-8b-q4km");
  const beforeDuplicates = await llmField.locator('option').allTextContents();
  await page.evaluate(() => {
    const f = window.modelDownloadFixture;
    const current = f.entries.find(item => item.modelName === "Qwen3-8B");
    f.entries.unshift({...current, packageId:"77777777-7777-4777-8777-777777777777", compatibility:null,
      download:{operationId:"88888888-8888-4888-8888-888888888888", recipeHash:"b".repeat(64)}});
    const q4 = f.entries.find(item => item.modelName === "Qwen3-4B");
    q4.download.recipeHash = "c".repeat(64);
    f.setVisible(false);
  });
  await llmField.waitFor({state:"hidden"});
  await page.evaluate(() => window.modelDownloadFixture.setVisible(true));
  await llmField.waitFor();
  await page.waitForFunction(() => document.querySelector('select[aria-label="議事録生成のモデル"]')?.value === "official:llama-b11533-qwen3-8b-q4km");
  check(JSON.stringify(await llmField.locator('option').allTextContents()) === JSON.stringify(beforeDuplicates), "duplicate installation changed the options");
  check(await inventory.getByText("Qwen 3 4B · Q4_K_M", {exact:true}).count() === 1, "Qwen3 4B was not added");
  check(await inventory.getByText("Qwen 3 8B · Q4_K_M", {exact:true}).count() === 1, "duplicate package appeared in inventory");
  check(await llmField.inputValue() === "official:llama-b11533-qwen3-8b-q4km", "duplicate package displaced the active model");
  await llmField.selectOption("official:llama-b11533-qwen3-4b-q4km");
  await page.locator('.model-selection-row').filter({has:llmField}).getByRole('button', {name:"使用",exact:true}).click();
  await approveModelLicense();
  await page.waitForFunction(() => {
    const f = window.modelDownloadFixture, id = f.profiles[0].components.find(c => c.engine === "llama.cpp").packageId;
    return f.entries.find(e => e.packageId === id)?.modelName === "Qwen3-4B";
  });
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "start_model_download").length) === 7, "older recipe downloaded identical model bytes again");
  results.push("duplicate packages and older recipe IDs reuse exact model bytes with one label and retain the active selection");
  await page.screenshot({ path: "output/playwright/model-choices.png" });
  return { passed: results.length, results };
})
