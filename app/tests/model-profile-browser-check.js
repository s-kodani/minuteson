// CLI-first regression checks using model-profile-browser-fixture.js.
async (page) => {
  const results = [];
  const check = (value, message) => { if (!value) throw Error(message); };
  const records = () => page.getByRole("region", { name: "モデル配置記録", exact: true }).getByRole("listitem");
  const modal = () => page.getByRole("dialog");
  const count = command => page.evaluate(c => window.modelProfileFixture.calls.filter(x => x.command === c).length, command);
  const boot = async () => {
    await page.evaluate(() => sessionStorage.removeItem("mon-model-profile-fixture"));
    await page.reload();
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await records().first().getByRole("button", { name: "標準への適用を確認", exact: true }).waitFor();
    await page.waitForFunction(() => !document.body.textContent.includes("配置とプロファイル構成を再読込しています。"));
  };
  const review = async (index = 0) => {
    await records().nth(index).getByRole("button", { name: "標準への適用を確認", exact: true }).click();
    await modal().getByRole("button", { name: "確認した内容で適用", exact: true }).waitFor();
    await page.waitForFunction(() => !document.body.textContent.includes("モデルとCPU試行証拠を再照合し、変更内容を準備しています。"));
  };
  await boot(); await review();
  check(await count("apply_model_profile_application") === 0, "application preview wrote settings");
  await modal().getByRole("button", { name: "適用せず閉じる", exact: true }).click();
  await modal().waitFor({ state: "hidden" });
  check(await records().first().getByRole("button", { name: "標準への適用を確認", exact: true }).evaluate(e => e === document.activeElement), "application focus not restored");
  results.push("existing application: preview, explicit non-apply close and focus");
  for (const [index, engine] of ["whisper.cpp", "sherpa-onnx", "llama.cpp"].entries()) {
    await review(index);
    await modal().getByRole("button", { name: "確認した内容で適用", exact: true }).click();
    await modal().waitFor({ state: "hidden" });
    await page.waitForFunction(() => !document.body.textContent.includes("配置とプロファイル構成を再読込しています。"));
    check(await count("apply_model_profile_application") === index + 1, "duplicate application");
    const preference = await page.evaluate(e => window.modelProfileFixture.preferences[e].payload.standard, engine);
    check(preference.threads === [7, 6, 5][index], "CPU tuning changed");
    if (index === 0) check(preference.useGpu === true, "ASR GPU setting changed");
    if (index === 1) check(preference.clusterThresholdMilli === 700 && preference.windowShiftMilli === 400, "diarization conditions changed");
    if (index === 2) check(preference.contextTokens === 8192 && preference.gpuLayers === 2 && preference.seed === 777, "generation conditions changed");
    results.push(`${engine}: explicit apply keeps numerical tuning`);
  }
  check(await page.evaluate(() => window.modelProfileFixture.profiles[0].configurationComplete), "complete profile selection lost");
  await boot(); await page.evaluate(() => window.modelProfileFixture.previewMode = "delay");
  await records().first().getByRole("button", { name: "標準への適用を確認", exact: true }).click();
  await page.getByRole("button", { name: "適用の事前確認を取り消す", exact: true }).click();
  await page.evaluate(() => window.modelProfileFixture.complete());
  await page.waitForFunction(() => !document.body.textContent.includes("取消要求済み。"));
  check(await modal().count() === 0 && await count("apply_model_profile_application") === 0, "late cancelled preview applied or resurrected");
  results.push("existing application: cancelled late preview remains suppressed");
  await boot(); await page.evaluate(() => window.modelProfileFixture.previewMode = "forged");
  await records().first().getByRole("button", { name: "標準への適用を確認", exact: true }).click();
  await page.getByRole("complementary", { name: "通知", exact: true }).getByRole("alert").filter({ hasText: "モデル準備" }).waitFor();
  check(await modal().count() === 0 && await count("apply_model_profile_application") === 0, "forged application preview accepted");
  results.push("existing application: forged tuning rejected");
  await boot(); await review(); await page.evaluate(() => window.modelProfileFixture.mode = "lost");
  await modal().getByRole("button", { name: "確認した内容で適用", exact: true }).click();
  await modal().getByRole("alert").filter({ hasText: "適用結果を確定できません" }).waitFor();
  await modal().getByRole("button", { name: "同じ操作で適用結果を確認", exact: true }).click();
  await modal().waitFor({ state: "hidden" });
  const applications = await page.evaluate(() => window.modelProfileFixture.calls.filter(c => c.command === "apply_model_profile_application"));
  check(applications.length === 2 && applications[0].args.token === applications[1].args.token && await page.evaluate(() => window.modelProfileFixture.profiles[0].profileVersion) === 1, "unknown reply duplicated profile version");
  results.push("existing application: lost reply recovers the same operation without duplicate revision");
  return { passed: results.length, results };
}
