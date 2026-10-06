// CLI-first synthetic IPC. No native deletion or actual model access.
async (page) => {
  page.setDefaultTimeout(10000);
  const results = [], check = (v, message) => { if (!v) throw Error(message); };
  const region = () => page.getByRole("region", { name: "モデル本体削除と中断対応", exact: true });
  const refresh = () => region().getByRole("button", { name: "モデル削除と書込停止の状態を再読込", exact: true });
  const review = () => region().getByRole("button", { name: "未完了モデル削除の対象を確認", exact: true });
  const dialog = () => page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: /モデル本体の削除を確認|モデル本体の残り範囲への再同意/u }) });
  const boot = async reason => {
    await page.reload(); await page.getByRole("button", { name: "設定", exact: true }).click();
    await page.getByRole("button", { name: "本体削除の実行確認へ", exact: true }).waitFor();
    await page.evaluate(r => {
      const d = window.modelBodyFixture;
      d.pending("awaiting_ack", true); d.pendingPreview.priorDeletionCompleted = true;
      Object.assign(d.items[0], { state: "completed", startupReason: r });
      if (r === "model_completed_receipt_invalid") Object.assign(d.items[0], { packageId: null, modelName: null, reviewAllowed: false, errorMessage: "模擬の破損履歴を保持" });
    }, reason);
    await refresh().click();
    await region().getByText(/完了記録あり・対象確認が必要/u).waitFor();
    check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "startup hint released normal writes");
    check(await page.evaluate(() => window.modelBodyFixture.deleted) === 0, "startup auto-deleted model");
  };
  await boot("model_completed_scope_reappeared");
  await region().getByText(/対象が再出現しています。自動では削除しません/u).waitFor();
  await review().click(); await dialog().waitFor(); await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check(await dialog().getByRole("button", { name: "照合してモデル本体削除を続行", exact: true }).isDisabled(), "old completion allowed changed scope");
  await page.evaluate(() => { window.modelBodyFixture.unknownCount = 1; });
  await dialog().getByRole("button", { name: "モデル本体の変更後の残りを確認（まだ削除しない）", exact: true }).click();
  await dialog().getByRole("heading", { name: "モデル本体の残り範囲への再同意", exact: true }).waitFor();
  await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check((await dialog().innerText()).includes("元の本体削除は完了済み"), "completed original was not explained");
  const publish = dialog().getByRole("button", { name: "新しいモデル範囲へ再同意して削除を続行", exact: true });
  await dialog().getByRole("checkbox", { name: /対象を確認し、モデル本体は/u }).check();
  check(await publish.isDisabled(), "reappearance adopted unknown without consent");
  await dialog().getByRole("checkbox", { name: /確認時の一覧にある未登録パスも/u }).check();
  await publish.click();
  await region().getByRole("status").filter({ hasText: "モデル本体を削除しました" }).waitFor();
  await region().getByText(/通常利用にはアプリの再起動が必要/u).waitFor();
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "completion skipped cold restart");
  check(await page.evaluate(() => window.dataFixture.meetings.length === 1 && window.modelBodyFixture.deleted === 1), "meeting modified or duplicated deletion");
  results.push("completed reappearance retains source meaning, requires new separate consent and restart");

  await boot("model_completed_receipt_invalid");
  await region().getByText(/記録・同意履歴を検証できません/u).waitFor();
  check(await review().isDisabled(), "invalid receipt granted review");
  await refresh().click();
  check(await review().isDisabled() && await page.evaluate(() => window.modelBodyFixture.deleted) === 0, "refresh repaired/removed invalid history implicitly");
  results.push("invalid completed receipt remains visible, retained and locked");

  await boot("model_completed_scope_uninspectable");
  await region().getByText(/保存領域・対象を安全に検査できません/u).waitFor();
  check(await page.evaluate(() => !window.dataFixture.calls.some(c => c.command === "start_data_operation")), "uninspectable startup published");
  await page.setViewportSize({ width: 900, height: 800 });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "startup explanation overflows horizontally");
  await page.screenshot({ path: "output/playwright/model-startup-attention-900.png" });
  results.push("uninspectable root remains retained with read-only review option and 900px layout");
  return { passed: results.length, results };
}
