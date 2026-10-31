// Synthetic model lifecycle and IPC; no real model files or user database writes.
async page => {
  page.setDefaultTimeout(10_000);
  const check = (ok, message) => { if (!ok) throw Error(message); }, results = [];
  await page.goto("http://127.0.0.1:1420/tests/model-download-browser.html?official=1");
  await page.setViewportSize({ width: 1280, height: 900 });
  const model = page.getByRole("combobox", { name: "話者分離のモデル", exact: true });
  await model.selectOption("official:sherpa-1138-pyannote3-eres2net-base");
  await page.locator(".model-selection-row").filter({ has: model }).getByRole("button", { name: "ダウンロードして使用", exact: true }).click();
  const cpu = page.getByRole("combobox", { name: "自動の話者統合", exact: true });
  const apply = page.getByRole("button", { name: "統合の強さを適用", exact: true });
  await cpu.waitFor(); await page.waitForFunction(() => document.querySelector('select[aria-label="自動の話者統合"]')?.disabled === false);
  const baseline = await page.evaluate(() => ({ settings: structuredClone(window.modelDownloadFixture.settingsByEngine), profile: structuredClone(window.modelDownloadFixture.profiles[0]), high: structuredClone(window.modelDownloadFixture.profiles[1]), calls: structuredClone(window.modelDownloadFixture.calls) }));
  await cpu.selectOption("1300");
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length) === 1, "selection alone wrote settings");
  await apply.click();
  await page.waitForFunction(() => window.modelDownloadFixture.settingsByEngine["sherpa-onnx"].clusterThresholdMilli === 1300 && document.querySelector('select[aria-label="自動の話者統合"]')?.value === "1300");
  const after = await page.evaluate(() => ({ settings: structuredClone(window.modelDownloadFixture.settingsByEngine), profile: structuredClone(window.modelDownloadFixture.profiles[0]), high: structuredClone(window.modelDownloadFixture.profiles[1]), calls: structuredClone(window.modelDownloadFixture.calls) }));
  const expected = structuredClone(baseline.settings); expected["sherpa-onnx"].clusterThresholdMilli = 1300;
  check(JSON.stringify(after.settings) === JSON.stringify(expected), "Automatic merge tuning changed model paths or quality settings");
  check(JSON.stringify(after.high) === JSON.stringify(baseline.high), "Automatic merge tuning changed the other profile");
  check(after.profile.profileVersion === baseline.profile.profileVersion + 1, "Automatic merge tuning did not create a profile revision");
  const writes = after.calls.slice(baseline.calls.length);
  check(writes.filter(c => c.command === "apply_model_profile_application").length === 1 && !writes.some(c => ["start_model_download", "check_model_compatibility", "save_diarization_preferences"].includes(c.command)), "Automatic merge tuning downloaded, ran a model or bypassed profile application");
  results.push("stronger automatic merging saves a new profile revision while retaining models, quality settings and the other profile");
  const style=await cpu.evaluate(element=>({appearance:getComputedStyle(element).appearance,height:element.getBoundingClientRect().height,font:getComputedStyle(element).fontSize}));
  check(style.appearance==="none"&&style.height===40&&style.font==="13px","automatic merge control is outside the shared design");
  await page.screenshot({ path: "output/playwright/diarization-auto-relaxed/merge-settings.png" });

  await cpu.selectOption("500"); await page.evaluate(() => { window.modelDownloadFixture.mode = "applyLost"; });
  await apply.click();
  const recheck = page.locator('.model-inspection>p[role="status"]');
  await recheck.waitFor();
  check(await cpu.isDisabled() && await apply.isDisabled(), "uncertain apply did not fence another write");
  const count = await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length);
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; }); await recheck.waitFor({ state: "hidden" });
  check(await cpu.inputValue() === "500" && await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length) === count, "lost response caused a repeated apply or failed to restore the previous threshold");
  results.push("rollback to the previous threshold and a lost response resolve by readback without duplicate application");

  await cpu.selectOption("1300"); await page.evaluate(() => { window.modelDownloadFixture.mode = "applyFailure"; });
  await apply.click(); await recheck.waitFor();
  check(await page.evaluate(() => window.modelDownloadFixture.settingsByEngine["sherpa-onnx"].clusterThresholdMilli) === 500, "failed save changed the current settings");
  const failedCount = await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length);
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; }); await recheck.waitFor({ state: "hidden" });
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(c => c.command === "apply_model_profile_application").length) === failedCount, "recheck repeated a failed write");
  await cpu.selectOption("1300"); await apply.click();
  await page.waitForFunction(() => window.modelDownloadFixture.settingsByEngine["sherpa-onnx"].clusterThresholdMilli === 1300);
  results.push("a failed save preserves current settings and requires an explicit apply to retry");

  await page.evaluate(() => window.modelDownloadFixture.setLocked(true));
  await page.waitForFunction(() => document.querySelector('select[aria-label="自動の話者統合"]')?.disabled === true);
  check(await cpu.isDisabled() && await apply.isDisabled(), "locked settings still allow automatic merge changes");
  await page.evaluate(() => { window.modelDownloadFixture.setLocked(false); window.modelDownloadFixture.setVisible(false); });
  await cpu.waitFor({ state: "hidden" }); await page.evaluate(() => window.modelDownloadFixture.setVisible(true));
  await cpu.waitFor(); await page.waitForFunction(() => document.querySelector('select[aria-label="自動の話者統合"]')?.value === "1300");
  await page.setViewportSize({ width: 390, height: 844 });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), "automatic merge settings overflow the narrow viewport");
  results.push("lock state disables changes and reopening settings reloads the saved value in a narrow viewport");
  return { passed: results.length, results };
}
