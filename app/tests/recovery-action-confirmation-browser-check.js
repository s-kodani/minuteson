async page => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const region = () => page.getByRole("region", { name: "中断したデータ操作の確認", exact: true });
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const count = command => page.evaluate(c => window.dataFixture.calls.filter(x => x.command === c).length, command);
  const boot = async (options = {}) => {
    await page.reload(); await page.waitForFunction(() => !!window.dataFixture);
    await page.evaluate(values => { const f = window.dataFixture; Object.assign(f, values); const id = crypto.randomUUID(); f.recoveryItems = [{ source: "scratch", operationId: id, paths: [`.mon-stage-${id}`], registeredPaths: 2, reason: "未公開物を保持", retained: false }]; }, options);
    await page.getByRole("button", { name: "設定", exact: true }).click(); await region().getByRole("button", { name: "対象と解決方法を確認", exact: true }).waitFor();
  };
  const source = () => region().getByRole("button", { name: "対象と解決方法を確認", exact: true });
  const reviewTitle = "未確定データの解決方法を確認", changeTitle = "回収対象の確認内容変更を確認", stopTitle = "回収操作の安全な停止を確認";
  const oldDataReview = async () => {
    await boot({ mode: "stale" }); const d = modal("会議保存先の変更確認");
    await page.getByRole("button", { name: "会議保存先を変更", exact: true }).click(); await d.getByRole("button", { name: "コピーして保存先を切替", exact: true }).click(); await d.waitFor({ state: "hidden" });
  };
  await oldDataReview(); const original = await page.evaluate(() => JSON.stringify(window.dataFixture.snapshot));
  await source().click(); await modal(changeTitle).waitFor(); check((await modal(changeTitle).innerText()).includes("検査だけでは回収・保持確認を行いません"), "review warning omitted");
  await page.keyboard.press("Escape"); await modal(changeTitle).waitFor({ state: "hidden" });
  check(await count("review_recovery_item") === 0 && await page.evaluate(() => JSON.stringify(window.dataFixture.snapshot)) === original, "cancel changed previous data review");
  await source().click(); await modal(changeTitle).getByRole("button", { name: "確認内容を破棄して対象を検査する", exact: true }).click(); await modal(changeTitle).waitFor({ state: "hidden" }); await modal(reviewTitle).waitFor();
  check(await count("review_recovery_item") === 1 && await count("start_data_operation") === 1 && await page.evaluate(() => window.dataFixture.recoveryItems[0].retained === false), "readonly inspection resolved recovery or repeated");
  results.push("changing another review into recovery requires consent; decline preserves exact state and one accepted inspection is read-only");

  await boot({ recoveryMode: "delay" }); await source().click(); await modal(reviewTitle).getByRole("button", { name: "保持して確認を完了", exact: true }).click();
  const token = await page.evaluate(() => window.dataFixture.snapshot.token);
  const stop = () => region().getByRole("button", { name: "安全な停止を要求", exact: true });
  await stop().click(); await modal(stopTitle).waitFor(); await page.keyboard.press("Escape"); await modal(stopTitle).waitFor({ state: "hidden" }); check(await count("cancel_data_operation") === 0, "declined stop cancelled recovery");
  await stop().click(); await modal(stopTitle).getByRole("button", { name: "安全な停止を要求して待つ", exact: true }).dblclick(); await page.waitForFunction(() => window.dataFixture.snapshot.phase === "cancelling");
  for (let i = 0; i < 5; i++) await page.keyboard.press("Escape");
  check(await modal(stopTitle).isVisible() && await count("cancel_data_operation") === 1 && await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "stop dismissed or repeated while recovery ran");
  await page.evaluate(() => window.dataFixture.complete()); await modal(stopTitle).waitFor({ state: "hidden" });
  check(await page.evaluate(t => window.dataFixture.calls.find(c => c.command === "cancel_data_operation").args.token === t && window.dataFixture.recoveryItems[0].retained === false, token), "stop rebound identity or falsely kept item");
  results.push("recovery stop requires consent and waits for same-token actual termination without falsely retaining or recovering the item");

  await boot({ recoveryMode: "delay", failCancel: true }); await source().click(); await modal(reviewTitle).getByRole("button", { name: "保持して確認を完了", exact: true }).click();
  await stop().click(); await modal(stopTitle).getByRole("button", { name: "安全な停止を要求して待つ", exact: true }).click(); await modal(stopTitle).getByRole("alert").waitFor();
  check(await count("cancel_data_operation") === 1 && await modal(stopTitle).getByRole("button", { name: "安全な停止を要求して待つ", exact: true }).isDisabled(), "failed stop replayed");
  await modal(stopTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click(); check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "failed stop unlocked recovery");
  await page.evaluate(() => window.dataFixture.complete());
  results.push("failed stop remains uncertain and native write protection survives closing the error; original work is not replayed");
  return { passed: results.length, results };
}
