// CLI-first browser verification, not a Playwright test spec. Run with
// playwright-cli -s=mon-rebinding run-code --filename <absolute path> after
// routing index.html to model-rebinding-browser-fixture.js in the local dev server.
async (page, capturePath, detailUI) => {
  const results = [];
  const check = (value, message) => { if (!value) throw Error(message); };
  const count = command => page.evaluate(c => window.rebindingFixture.calls.filter(x => x.command === c).length, command);
  const fixture = values => page.evaluate(v => Object.assign(window.rebindingFixture, v), values);
  const boot = async () => {
    await page.reload();
    await page.getByRole("button", { name: "開く", exact: true }).first().click(); await detailUI.openMeetingTools();
    await page.getByRole("list", { name: "解析ジョブ", exact: true }).waitFor();
  };
  const prepare = async (list = "解析ジョブ", packageId = "11111111-1111-4111-8111-111111111111") => {
    if (list === "議事録生成ジョブ") await detailUI.openMinutes();
    await page.getByRole("list", { name: list, exact: true }).getByRole("button", { name: "再配置モデルで同条件の再試行を確認" }).click();
    await page.getByRole("combobox", { name: "再配置先モデル" }).selectOption(packageId);
    await page.getByRole("button", { name: "元の固定条件と配置先を照合" }).click();
    await page.getByRole("button", { name: "この固定条件で再試行を開始" }).waitFor();
  };
  const apply = () => page.getByRole("button", { name: "この固定条件で再試行を開始" }).click();
  const receipt = () => page.getByRole("button", { name: "同じ操作IDで結果だけ確認" }).click();
  const closed = () => page.getByRole("dialog").waitFor({ state: "hidden" });
  for (const [list, prefix] of [["解析ジョブ", "1"], ["話者分離ジョブ", "2"], ["議事録生成ジョブ", "3"]]) {
    await boot(); await prepare(list, `${prefix}1111111-1111-4111-8111-111111111111`);
    check(await count("apply_job_model_rebinding") === 0, "preview enqueued work");
    const body = await page.getByRole("dialog").innerText();
    check(body.includes("保存済み 1 / 3") && body.includes("high_accuracy"), "fixed condition review missing");
    if (prefix === "2") check(body.includes('"numSpeakers": 3') && body.includes('"numSpeakers": null'), "track conditions substituted");
    await apply(); await closed();
    check(await count("apply_job_model_rebinding") === 1, "duplicate admission");
    check(await page.evaluate(() => window.rebindingFixture.jobs.length) === 4, "retry count mismatch");
    results.push(`${list}: explicit review and single admission`);
  }
  await boot(); await prepare(); await fixture({ mode: "lost" }); await apply();
  await page.getByRole("alert").filter({ hasText: "受付結果を確定できません" }).waitFor();
  await receipt(); await closed();
  check(await count("apply_job_model_rebinding") === 1 && await count("get_job_model_rebinding_result") === 1, "receipt enqueued another retry");
  results.push("lost committed reply: read-only receipt, no duplicate");

  await boot(); await prepare(); await fixture({ mode: "conflict" }); await apply(); await receipt();
  await page.getByRole("alert").filter({ hasText: "まだ確認できません" }).waitFor();
  check(await count("apply_job_model_rebinding") === 1, "absent receipt enqueued work");
  await fixture({ mode: "success" }); await page.getByRole("button", { name: "同じ操作で受付を再試行" }).click(); await closed();
  const attempts = await page.evaluate(() => window.rebindingFixture.calls.filter(c => c.command === "apply_job_model_rebinding").map(c => c.args));
  check(JSON.stringify(attempts[0]) === JSON.stringify(attempts[1]), "unknown identity silently replaced");
  results.push("absent receipt: explicit same-operation reapply");

  await boot(); await prepare(); await fixture({ mode: "lostActive" }); await apply(); await receipt();
  await page.getByRole("alert").filter({ hasText: "まだ終了していません" }).waitFor();
  check(await count("apply_job_model_rebinding") === 1, "active receipt retried admission");
  await page.evaluate(() => window.rebindingFixture.complete()); await receipt(); await closed();
  results.push("active unknown operation: keep identity until committed receipt");

  await boot(); await prepare(); await fixture({ mode: "cancelled" }); await apply();
  await page.getByRole("button", { name: "照合・受付処理の取消を要求" }).click();
  await page.keyboard.press("Escape"); check(await page.getByRole("dialog").isVisible(), "cancel closed before native completion");
  await page.evaluate(() => window.rebindingFixture.complete());
  await closed(); await page.getByRole("status").filter({ hasText: "受付取消と処理終了を確認" }).waitFor();
  check(await page.evaluate(() => window.rebindingFixture.jobs.length) === 3, "cancel committed work");
  results.push("cancelled admission: await completion, cancelled inactive receipt permits fresh review");

  await boot(); await prepare(); await fixture({ mode: "delay" }); await apply();
  await page.getByRole("button", { name: "照合・受付処理の取消を要求" }).click();
  await page.evaluate(() => window.rebindingFixture.complete()); await closed();
  check(await page.evaluate(() => window.rebindingFixture.jobs.length) === 4, "winning commit receipt suppressed");
  const host = page.getByRole("complementary", { name: "通知", exact: true });
  await host.getByRole("alert").filter({ hasText: "取消要求より先に" }).waitFor();
  await host.getByRole("button", { name: "注意通知を閉じる", exact: true }).click();
  check(await page.getByLabel("取消後のモデル再試行受付の要対応状態", { exact: true }).isVisible(), "dismissal erased winning commit attention");
  results.push("commit/cancel race: winning commit remains visible");

  await boot(); await fixture({ previewMode: "delay" });
  await page.getByRole("list", { name: "解析ジョブ", exact: true }).getByRole("button", { name: "再配置モデルで同条件の再試行を確認" }).click();
  await page.waitForFunction(() => document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled === false);
  check(await page.getByRole("dialog").getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "safe initial cancellation focus missing");
  await page.getByRole("combobox", { name: "再配置先モデル" }).selectOption("11111111-1111-4111-8111-111111111111");
  await page.getByRole("button", { name: "元の固定条件と配置先を照合" }).click();
  await page.getByRole("button", { name: "照合・受付処理の取消を要求" }).click();
  await page.evaluate(() => window.rebindingFixture.complete());
  await page.waitForFunction(() => !document.body.textContent.includes("取消要求済み。"));
  check(await page.getByRole("button", { name: "この固定条件で再試行を開始" }).count() === 0, "late preview resurrected");
  await page.keyboard.press("Escape"); await closed();
  check(await page.getByRole("list", { name: "解析ジョブ", exact: true }).getByRole("button", { name: "再配置モデルで同条件の再試行を確認" }).evaluate(e => e === document.activeElement), "focus not restored");
  results.push("late cancelled preview: no resurrection; Escape and focus restoration");

  await boot(); await prepare(); await fixture({ mode: "lost", failReceipt: true }); await apply(); await receipt();
  await page.getByRole("alert").filter({ hasText: "結果を確認できません" }).waitFor();
  await page.getByRole("dialog").getByRole("button", { name: "閉じる", exact: true }).click();
  await page.getByRole("button", { name: "開く", exact: true }).nth(1).click(); await detailUI.openMeetingTools();
  await page.getByRole("button", { name: "開く", exact: true }).first().click(); await detailUI.openMeetingTools();
  await page.getByRole("button", { name: "モデル再試行の受付結果を確認" }).click();
  await fixture({ failReceipt: false }); await receipt(); await closed();
  check(await count("apply_job_model_rebinding") === 1, "meeting switch replaced unknown operation");
  results.push("receipt failure + meeting switch: recover retained operation");

  await boot(); await fixture({ previewMode: "forged" });
  await page.getByRole("list", { name: "解析ジョブ", exact: true }).getByRole("button", { name: "再配置モデルで同条件の再試行を確認" }).click();
  await page.getByRole("combobox", { name: "再配置先モデル" }).selectOption("11111111-1111-4111-8111-111111111111");
  await page.getByRole("button", { name: "元の固定条件と配置先を照合" }).click();
  await page.getByRole("alert").filter({ hasText: "配置証拠が一致しません" }).waitFor();
  check(await count("apply_job_model_rebinding") === 0, "forged proof admitted");
  results.push("forged dependency proof: rejected before admission");

  await boot(); await prepare(); await fixture({ mode: "forged" }); await apply();
  await page.getByRole("alert").filter({ hasText: "受付結果を確定できません" }).waitFor();
  await receipt(); await closed();
  check(await count("apply_job_model_rebinding") === 1, "mismatched reply generated new operation");
  results.push("mismatched admission reply: reject and recover authoritative receipt");

  await boot(); await prepare(); await fixture({ mode: "conflict" }); await apply();
  await page.getByRole("button", { name: "未確定の受付を取り消して確認" }).click();
  await closed(); await page.getByRole("status").filter({ hasText: "受付取消と処理終了を確認" }).waitFor();
  await page.getByRole("list", { name: "解析ジョブ", exact: true }).getByRole("button", { name: "再配置モデルで同条件の再試行を確認" }).click();
  check(await page.getByRole("combobox", { name: "再配置先モデル" }).inputValue() === "", "cancel retained target as implicit selection");
  check(await count("apply_job_model_rebinding") === 1 && await page.evaluate(() => window.rebindingFixture.jobs.length) === 3, "unknown cancellation admitted work");
  results.push("unknown absent admission: explicit cancellation and fresh target review");

  await boot(); await fixture({ failCatalog: true });
  await page.getByRole("list", { name: "解析ジョブ", exact: true }).getByRole("button", { name: "再配置モデルで同条件の再試行を確認" }).click();
  await page.getByRole("alert").filter({ hasText: "模擬配置読込失敗" }).waitFor();
  check(await page.getByRole("button", { name: "元の固定条件と配置先を照合" }).isDisabled(), "catalog failure allowed preview");
  await fixture({ failCatalog: false }); await page.getByRole("button", { name: "配置記録を再読込" }).click();
  await page.waitForFunction(() => document.querySelector("dialog[open] select")?.options.length === 2);
  check(await page.getByRole("combobox", { name: "再配置先モデル" }).inputValue() === "", "catalog reload auto-selected target");
  results.push("catalog read failure: no automatic target; explicit reload recovers");

  await boot(); await fixture({ failSave: true });
  await page.getByText("エンジン設定（端末内に保存）", { exact: true }).click();
  await page.getByRole("spinbutton", { name: "CPUスレッド数", exact: true }).fill("6");
  await page.getByRole("list", { name: "解析ジョブ", exact: true }).getByRole("button", { name: "再配置モデルで同条件の再試行を確認" }).click();
  await page.getByRole("combobox", { name: "再配置先モデル" }).selectOption("11111111-1111-4111-8111-111111111111");
  await page.getByRole("button", { name: "元の固定条件と配置先を照合" }).click();
  await page.getByRole("alert").filter({ hasText: "編集の保存を完了できません" }).waitFor();
  check(await count("preview_job_model_rebinding") === 0, "save failure started preflight");
  results.push("pending edit save failure: preserve input and never start preflight");
  await page.keyboard.press("Escape");
  return { passed: results.length, results };
}
