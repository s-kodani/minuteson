// Synthetic model metadata and IPC only. No downloads, native AI or user data.
async page => {
  const check = (value, message) => { if (!value) throw Error(message); };
  const results = [];
  const models = () => page.getByRole("region", { name: "AIモデル", exact: true });
  const jobs = () => page.getByRole("region", { name: "バックグラウンドのジョブ一覧", exact: true });
  const common = () => page.getByRole("region", { name: "進行中の処理", exact: true });
  const row = () => models().locator(".model-selection-row").filter({ has: page.getByRole("combobox", { name: "議事録生成のモデル", exact: true }) });
  const count = command => page.evaluate(c => window.modelDownloadFixture.calls.filter(call => call.command === c).length, command);
  const boot = async mode => {
    await page.goto("http://127.0.0.1:1420/tests/operation-progress-browser.html?official=1");
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await models().getByRole("combobox", { name: "議事録生成のモデル", exact: true }).selectOption("official:llama-b11533-qwen3-8b-q4km");
    await page.evaluate(mode => window.modelDownloadFixture.mode = mode, mode);
    await row().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  };

  await boot("placeFailure");
  await models().getByRole("button", { name: "現在の状態を確認", exact: true }).waitFor();
  await common().getByText("モデル準備 · 状態の確認が必要", { exact: true }).waitFor();
  await page.getByRole("button", { name: "ジョブ一覧", exact: true }).click();
  await jobs().getByText("Qwen 3 8B · Q4_K_M", { exact: true }).waitFor();
  await jobs().getByText("中断", { exact: true }).waitFor();
  await jobs().getByRole("button", { name: "モデル設定を開く", exact: true }).click();
  await page.evaluate(() => window.modelDownloadFixture.mode = "success");
  await models().getByRole("button", { name: "現在の状態を確認", exact: true }).click();
  await models().getByRole("button", { name: "現在の状態を確認", exact: true }).waitFor({ state: "hidden" });
  await row().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await models().getByRole("region", { name: "導入済みモデル", exact: true }).getByText("使用中", { exact: true }).waitFor();
  await common().waitFor({ state: "hidden" });
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 2 && await count("apply_model_profile_application") === 1, "recovery redownloaded or skipped the explicit retry");
  check(await count("get_downloaded_model_placement_result") === 1, "failed placement was retried before its original result was fenced");
  results.push("Qwen download survives placement failure; interrupted job remains accessible and resumes without restart or redownload");

  await boot("delay");
  await page.waitForFunction(() => window.modelDownloadFixture.active.size === 1);
  await page.evaluate(() => window.modelDownloadFixture.finish());
  await common().getByText("モデルを配置中", { exact: true }).waitFor();
  const token = await page.evaluate(() => window.modelDownloadFixture.calls.findLast(call => call.command === "place_downloaded_model").args.token);
  await page.evaluate(token => window.operationProgressFixture.emitModelWait(token, true), token);
  await common().getByText("モデル準備 · AI処理の完了待ち", { exact: true }).waitFor();
  await page.getByRole("button", { name: "ジョブ一覧", exact: true }).click();
  await jobs().getByText("待機中", { exact: true }).waitFor();
  await page.evaluate(() => window.operationProgressFixture.emitModelWait(crypto.randomUUID(), false));
  check(await jobs().getByText("モデル準備 · AI処理の完了待ち", { exact: true }).isVisible(), "foreign wait event changed the current operation");
  await page.screenshot({ path: "output/playwright/model-preparation-waiting.png" });
  await page.evaluate(token => window.operationProgressFixture.emitModelWait(token, false), token);
  await jobs().getByText("モデルを配置中", { exact: true }).waitFor();
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; window.modelDownloadFixture.finish(); });
  await common().waitFor({ state: "hidden" });
  await jobs().getByText("進行中のジョブはありません", { exact: true }).waitFor();
  check(await count("start_model_download") === 1 && await count("apply_model_profile_application") === 1, "waiting interrupted setup or repeated download");
  results.push("operation-bound waiting status persists across navigation, ignores foreign events and clears after setup");
  return { passed: results.length, results };
}
