async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const host = () => page.getByRole("complementary", { name: "通知", exact: true });
  const panel = () => page.getByRole("region", { name: "AIモデルの事前検査", exact: true });
  const modal = () => page.getByRole("dialog");
  const count = command => page.evaluate(c => window.modelPreparationFixture.calls.filter(x => x.command === c).length, command);
  const boot = async () => { await page.reload(); await page.getByRole("button", { name: "設定", exact: true }).click(); await panel().getByRole("button", { name: "互換性を確認", exact: true }).waitFor(); };
  const inspect = async () => { await panel().getByRole("textbox", { name: "パッケージフォルダーの絶対パス", exact: true }).fill("/synthetic/model-package"); await panel().getByRole("button", { name: "ファイルを事前検査", exact: true }).click(); await panel().getByRole("button", { name: "管理領域へコピー", exact: true }).waitFor(); };
  const executeCpu = async () => { await panel().getByRole("button", { name: "互換性を確認", exact: true }).click(); await modal().getByRole("button", { name: "実行ファイルを起動して確認する", exact: true }).click(); };
  await boot(); await panel().getByRole("button", { name: "配置・構成記録を再読込", exact: true }).click();
  check(await host().getByRole("status").count() === 0, "catalog history emitted completion");
  results.push("historical catalog and routine reads do not emit model completion");

  await inspect(); await host().getByRole("status").filter({ hasText: "情報：" }).waitFor();
  check(await count("place_model_package") === 0 && await count("check_model_compatibility") === 0, "preflight started placement or model execution");
  const first = await host().locator(".toast-card").getAttribute("data-toast-id");
  await host().getByRole("button", { name: "情報通知を閉じる", exact: true }).click();
  check(await panel().getByRole("button", { name: "管理領域へコピー", exact: true }).isEnabled(), "dismissal cleared reviewed files");
  await panel().getByRole("button", { name: "ファイルを事前検査", exact: true }).click(); await host().getByRole("status").filter({ hasText: "事前検査が完了" }).waitFor();
  check(await host().locator(".toast-card").getAttribute("data-toast-id") !== first && await count("inspect_model_package") === 2, "new inspection suppressed as old wording");
  results.push("preflight is information only; dismiss keeps review and a new exact operation is notified");

  await panel().getByRole("button", { name: "管理領域へコピー", exact: true }).click();
  check(await count("place_model_package") === 0 && await host().getByRole("status").filter({ hasText: "コピーしました" }).count() === 0, "copy consent treated as completion");
  await modal().getByRole("button", { name: "管理領域へコピーする", exact: true }).click(); await modal().waitFor({ state: "hidden" });
  await host().getByRole("status").filter({ hasText: "成功：" }).waitFor(); await host().getByRole("button", { name: "成功通知を閉じる", exact: true }).click();
  await panel().getByRole("button", { name: "配置・構成記録を再読込", exact: true }).click();
  check(await count("place_model_package") === 1 && await host().getByRole("status").filter({ hasText: "コピーしました" }).count() === 0 && await panel().locator('[aria-label="モデル準備の結果"]').count() === 1, "dismiss/read repeated copy or removed retained result");
  results.push("verified placement completes once; dismiss and read keep status without recopy or re-notification");

  for (const mode of ["lost", "forged"]) {
    await boot(); await inspect(); await host().getByRole("button", { name: "情報通知を閉じる", exact: true }).click();
    await page.evaluate(mode => window.modelPreparationFixture.mode = mode, mode);
    await panel().getByRole("button", { name: "管理領域へコピー", exact: true }).click(); await modal().getByRole("button", { name: "管理領域へコピーする", exact: true }).click(); await modal().getByRole("alert").waitFor();
    check(await host().getByRole("status").count() === 0 && await host().getByRole("alert").count() === 0, "modal copy failure emitted outside success or error");
    await modal().getByRole("button", { name: "確認を閉じる", exact: true }).click(); await host().getByRole("alert").filter({ hasText: "モデル準備" }).waitFor();
    await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
    check(await panel().getByLabel("モデル準備の要対応状態", { exact: true }).count() === 1 && await count("place_model_package") === 1, "error dismissal lost uncertainty or repeated copy");
    results.push(`${mode} placement stays modal then retained issue, never success or automatic repeat`);
  }

  await boot(); await executeCpu(); await modal().waitFor({ state: "hidden" }); await host().getByRole("status").filter({ hasText: "合成入力でCPU互換性" }).waitFor();
  await panel().getByRole("button", { name: "モデル準備の結果の説明を開く", exact: true }).click();
  check((await page.getByRole("dialog").innerText()).includes("プロファイルは変更していません") && (await page.getByRole("dialog").innerText()).includes("未評価"), "CPU result claimed production readiness");
  await page.keyboard.press("Escape"); await host().getByRole("button", { name: "成功通知を閉じる", exact: true }).click();
  check(await count("check_model_compatibility") === 1, "CPU notification triggered execution");
  results.push("CPU success is fixed synthetic input only, with retained unapplied and unmeasured details");

  await boot(); await page.evaluate(() => window.modelPreparationFixture.mode = "failed"); await executeCpu(); await modal().getByRole("alert").waitFor();
  await modal().getByRole("button", { name: "確認を閉じる", exact: true }).click(); await host().getByRole("alert").filter({ hasText: "CPU互換性確認は失敗" }).waitFor();
  check(await host().getByRole("status").count() === 0 && await count("check_model_compatibility") === 1, "CPU failure announced success or retried execution");
  results.push("CPU failure retains cause and records without success or unsafe toast retry");

  await boot(); await page.evaluate(() => window.modelPreparationFixture.mode = "delay"); await executeCpu();
  await modal().getByRole("button", { name: "準備の停止を要求する", exact: true }).click();
  check(await host().getByRole("status").count() === 0, "stop request alone emitted cancellation completion");
  await page.evaluate(() => window.modelPreparationFixture.complete()); await modal().waitFor({ state: "hidden" });
  await host().getByRole("status").filter({ hasText: "情報：" }).filter({ hasText: "CPU互換性確認を取り消しました" }).waitFor();
  check(await count("check_model_compatibility") === 1, "cancelled CPU trial reran");
  results.push("CPU cancellation information requires terminal same-operation receipt, not a stop request");

  await boot(); await page.evaluate(() => window.modelPreparationFixture.failInspect = true);
  await panel().getByRole("textbox", { name: "パッケージフォルダーの絶対パス", exact: true }).fill("/synthetic/invalid"); await panel().getByRole("button", { name: "ファイルを事前検査", exact: true }).click();
  await host().getByRole("alert").filter({ hasText: "模擬事前検査失敗" }).waitFor(); await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await panel().getByRole("textbox", { name: "パッケージフォルダーの絶対パス", exact: true }).inputValue() === "/synthetic/invalid" && await panel().getByLabel("モデル準備の要対応状態", { exact: true }).count() === 1, "failed preflight dismissal discarded input or cause");
  results.push("failed preflight preserves exact path and retained cause after toast dismissal");
  await page.screenshot({ path: "model-preparation-toast-error.png" });
  return { passed: results.length, results };
}
