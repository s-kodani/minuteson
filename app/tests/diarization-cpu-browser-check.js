// Synthetic model lifecycle and IPC; no real model files or user database writes.
async (page, capturePath, { approveModelLicense }) => {
  page.setDefaultTimeout(10_000);
  const check = (ok, message) => { if (!ok) throw Error(message); }, results = [];
  await page.goto("http://127.0.0.1:1420/tests/model-download-browser.html?official=1");
  await page.setViewportSize({ width: 1280, height: 900 });
  const model = page.getByRole("combobox", { name: "話者分離のモデル", exact: true });
  await model.selectOption("official:sherpa-1138-pyannote3-eres2net-base");
  await page.locator(".model-selection-row").filter({ has: model }).getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  await approveModelLicense();
  const cpu = page.getByRole("combobox", { name: "話者分離のCPU使用量", exact: true });
  const apply = page.getByRole("button", { name: "CPU使用量を適用", exact: true });
  await cpu.waitFor(); await page.waitForFunction(() => document.querySelector('select[aria-label="話者分離のCPU使用量"]')?.disabled === false);
  const baseline = await page.evaluate(() => ({ settings: structuredClone(window.modelDownloadFixture.settingsByEngine), profile: structuredClone(window.modelDownloadFixture.profiles[0]), high: structuredClone(window.modelDownloadFixture.profiles[1]), calls: structuredClone(window.modelDownloadFixture.calls) }));
  await cpu.selectOption("8");
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length) === 1, "selection alone wrote settings");
  await apply.click();
  await page.waitForFunction(() => window.modelDownloadFixture.settingsByEngine["sherpa-onnx"].threads === 8 && document.querySelector('select[aria-label="話者分離のCPU使用量"]')?.value === "8");
  const after = await page.evaluate(() => ({ settings: structuredClone(window.modelDownloadFixture.settingsByEngine), profile: structuredClone(window.modelDownloadFixture.profiles[0]), high: structuredClone(window.modelDownloadFixture.profiles[1]), calls: structuredClone(window.modelDownloadFixture.calls) }));
  const expected = structuredClone(baseline.settings); expected["sherpa-onnx"].threads = 8;
  check(JSON.stringify(after.settings) === JSON.stringify(expected), "CPU tuning changed model paths or quality settings");
  check(JSON.stringify(after.high) === JSON.stringify(baseline.high), "CPU tuning changed the other profile");
  check(after.profile.profileVersion === baseline.profile.profileVersion + 1, "CPU tuning did not create a profile revision");
  const writes = after.calls.slice(baseline.calls.length);
  check(writes.filter(c => c.command === "apply_model_profile_application").length === 1 && !writes.some(c => ["start_model_download", "check_model_compatibility", "save_diarization_preferences"].includes(c.command)), "CPU tuning downloaded, ran a model or bypassed profile application");
  results.push("8-thread selection saves a new profile revision while retaining models, quality settings and the other profile");
  await page.screenshot({ path: "output/playwright/diarization-performance/cpu-settings.png" });

  await cpu.selectOption("4"); await page.evaluate(() => { window.modelDownloadFixture.mode = "applyLost"; });
  await apply.click();
  const recheck = page.locator('.model-inspection>p[role="status"]');
  await recheck.waitFor();
  check(await cpu.isDisabled() && await apply.isDisabled(), "uncertain apply did not fence another write");
  const count = await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length);
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; }); await recheck.waitFor({ state: "hidden" });
  check(await cpu.inputValue() === "4" && await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length) === count, "lost response caused a repeated apply or failed to restore 4 threads");
  results.push("4-thread rollback and a lost response resolve by readback without duplicate application");

  await cpu.selectOption("8"); await page.evaluate(() => { window.modelDownloadFixture.mode = "applyFailure"; });
  await apply.click(); await recheck.waitFor();
  check(await page.evaluate(() => window.modelDownloadFixture.settingsByEngine["sherpa-onnx"].threads) === 4, "failed save changed the current settings");
  const failedCount = await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length);
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; }); await recheck.waitFor({ state: "hidden" });
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length) === failedCount, "recheck repeated a failed write");
  await cpu.selectOption("8"); await apply.click();
  await page.waitForFunction(() => window.modelDownloadFixture.settingsByEngine["sherpa-onnx"].threads === 8);
  results.push("a failed save preserves current settings and requires an explicit apply to retry");

  await page.evaluate(() => window.modelDownloadFixture.setLocked(true));
  await page.waitForFunction(() => document.querySelector('select[aria-label="話者分離のCPU使用量"]')?.disabled === true);
  check(await cpu.isDisabled() && await apply.isDisabled(), "locked settings still allow CPU changes");
  await page.evaluate(() => { window.modelDownloadFixture.setLocked(false); window.modelDownloadFixture.setVisible(false); });
  await cpu.waitFor({ state: "hidden" }); await page.evaluate(() => window.modelDownloadFixture.setVisible(true));
  await cpu.waitFor(); await page.waitForFunction(() => document.querySelector('select[aria-label="話者分離のCPU使用量"]')?.value === "8");
  await page.setViewportSize({ width: 390, height: 844 });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), "CPU settings overflow the narrow viewport");
  results.push("lock state disables changes and reopening settings reloads the saved value in a narrow viewport");
  return { passed: results.length, results };
}
