// Existing recovery operation/receipts, simulated IPC. No real file removal.
async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const host = page.getByRole("complementary", { name: "通知", exact: true });
  const region = page.getByRole("region", { name: "中断したデータ操作の確認", exact: true });
  const modal = page.getByRole("dialog", { name: "未確定データの解決方法を確認", exact: true });
  const count = () => page.evaluate(() => window.dataFixture.calls.filter(c => c.command === "start_data_operation").length);
  const boot = async mode => {
    await page.reload(); await page.waitForFunction(() => !!window.dataFixture);
    await page.evaluate(value => { const f = window.dataFixture, id = crypto.randomUUID(); f.recoveryMode = value; f.recoveryItems = [{ source: "scratch", operationId: id, paths: [`.mon-stage-${id}`], registeredPaths: 2, reason: "未公開物を保持", retained: false }]; }, mode);
    await page.getByRole("button", { name: "設定", exact: true }).click(); await region.getByRole("button", { name: "対象と解決方法を確認", exact: true }).waitFor();
  };
  const review = async () => { await region.getByRole("button", { name: "対象と解決方法を確認", exact: true }).click(); await modal.waitFor(); };
  await boot("success"); await review();
  check(await host.locator(".toast-success").count() === 0 && await count() === 0, "inspection announced completed recovery");
  await modal.getByRole("button", { name: "保持して確認を完了", exact: true }).click();
  await host.getByRole("status").filter({ hasText: "回収はしていません" }).waitFor();
  const receipt = await page.evaluate(() => JSON.stringify(window.dataFixture.snapshot.recoveryReceipt));
  await host.getByRole("button", { name: "成功通知を閉じる", exact: true }).click();
  await region.getByRole("button", { name: "要確認一覧を再読込", exact: true }).click(); await page.waitForTimeout(1200);
  check(await host.locator(".toast-success").count() === 0 && await count() === 1 && await page.evaluate(r => JSON.stringify(window.dataFixture.snapshot.recoveryReceipt) === r, receipt), "dismissal or read replayed keep or erased receipt");
  check(await region.getByRole("status").filter({ hasText: "保持して確認済み（回収なし）" }).isVisible(), "dismissed keep erased the retained non-recovery state");
  results.push("inspection is not completion; confirmed keep notifies non-recovery and retains exact receipt and short state after dismissal/read without mutation");

  await boot("attention"); await review(); await modal.getByRole("button", { name: "再検査して回収", exact: true }).click();
  await host.getByRole("alert").filter({ hasText: "要確認が残っています" }).waitFor();
  check(await host.locator(".toast-success").count() === 0 && await region.getByRole("status").filter({ hasText: "未確認 1 件" }).isVisible(), "remaining attention was treated as successful recovery");
  await host.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await page.getByLabel("要確認データの確認・回収の要対応状態", { exact: true }).isVisible() && await count() === 1, "attention dismissal resolved issue or retried destructive work");
  results.push("attention receipt creates important problem, never success; dismiss retains issue and target without destructive action");

  await boot("lost"); await review(); await modal.getByRole("button", { name: "保持して確認を完了", exact: true }).click();
  await host.getByRole("status").filter({ hasText: "回収はしていません" }).waitFor();
  await page.waitForTimeout(1200);
  check(await host.getByRole("alert").filter({ hasText: "受付結果が未確定" }).count() === 0 && await count() === 1, "confirmed lost reply retained false uncertainty or resubmitted keep");
  results.push("lost accepted reply is resolved by matching operation receipt with no duplicate keep or stale uncertainty notice");

  await boot("delay"); await review(); await modal.getByRole("button", { name: "保持して確認を完了", exact: true }).click();
  await page.waitForFunction(() => !!window.dataFixture.complete);
  check(await host.locator(".toast-success").count() === 0, "running operation notified success before receipt");
  await page.evaluate(() => window.dataFixture.complete()); await host.getByRole("status").filter({ hasText: "回収はしていません" }).waitFor();
  results.push("busy recovery stays inline and only its verified terminal receipt creates completion notification");
  return { passed: results.length, results };
}
