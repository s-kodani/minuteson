// Synthetic model metadata and IPC only. No downloads, native AI or user data.
async (page, capturePath, { approveModelLicense }) => {
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
    await page.evaluate(mode => { window.modelDownloadFixture.mode = mode; window.modelDownloadFixture.failCancel = mode === "placeFailure"; }, mode);
    await row().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await approveModelLicense();
  };

  await boot("placeFailure");
  const recovering = () => models().locator(':scope > p[role="status"]');
  await recovering().waitFor();
  const disclosure = common().locator('.progress-disclosure');
  if (await disclosure.getAttribute('aria-expanded') !== 'true') await disclosure.click();
  await common().getByText("モデル準備 · 再試行を準備中", { exact: true }).waitFor();
  await page.getByRole("button", { name: "ジョブ一覧", exact: true }).click();
  await jobs().getByText("Qwen 3 8B · Q4_K_M", { exact: true }).waitFor();
  await jobs().getByText("処理中", { exact: true }).waitFor();
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; window.modelDownloadFixture.failCancel = false; });
  await common().waitFor({ state: "hidden" });
  await page.getByRole("button", { name: "設定", exact: true }).click();
  await row().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await approveModelLicense();
  await row().getByRole("button", { name: "使用中", exact: true }).waitFor();
  await common().waitFor({ state: "hidden" });
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 2 && await count("apply_model_profile_application") === 1, "recovery redownloaded or skipped the explicit retry");
  check(await page.evaluate(() => {
    const placements = window.modelDownloadFixture.calls.filter(call => call.command === "place_downloaded_model");
    return placements.every(call => typeof call.args.historyOperationId === "string") && placements[0].args.historyOperationId !== placements[1].args.historyOperationId;
  }), "explicit model retry reused the previous history operation");
  check(await count("get_downloaded_model_placement_result") === 1, "failed placement was retried before its original result was fenced");
  results.push("placement failure reconciles in the background after navigating away; retry reuses the Qwen download");

  await boot("movedFailure");
  await common().waitFor({ state: "hidden" });
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; });
  await row().getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await approveModelLicense();
  await row().getByRole("button", { name: "使用中", exact: true }).waitFor();
  const retries = await page.evaluate(() => window.modelDownloadFixture.calls.filter(call => call.command === "place_downloaded_model").map(call => call.args));
  check(await count("start_model_download") === 1 && retries.length === 2, "moved payload was downloaded again");
  check(retries[0].packageId === retries[1].packageId && retries[0].token !== retries[1].token, "resume did not keep its placement ID with a fresh cancellation token");
  check(await count("read_model_download") === 0, "resume tried to read the missing staging payload");
  results.push("interrupted move resumes its persistent placement with a fresh token, without reading or downloading the old payload");

  await boot("delay");
  await page.waitForFunction(() => window.modelDownloadFixture.active.size === 1);
  if (await disclosure.getAttribute('aria-expanded') !== 'true') await disclosure.click();
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
  await page.evaluate(() => window.modelDownloadFixture.verify());
  await jobs().getByText("配置したモデルを検証中", { exact: true }).waitFor();
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; window.modelDownloadFixture.finish(); });
  await common().waitFor({ state: "hidden" });
  await jobs().getByText("進行中のジョブはありません", { exact: true }).waitFor();
  check(await count("start_model_download") === 1 && await count("apply_model_profile_application") === 1, "waiting interrupted setup or repeated download");
  results.push("operation-bound waiting status persists across navigation, ignores foreign events and clears after setup");
  return { passed: results.length, results };
}
