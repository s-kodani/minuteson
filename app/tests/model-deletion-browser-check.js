// CLI-first browser check. Use a named Playwright CLI session routed to the
// panel-only synthetic fixture, then run-code --filename this file.
async (page) => {
  const results = [];
  const check = (value, message) => { if (!value) throw Error(message); };
  const trigger = () => page.getByRole("button", { name: "削除対象を確認（まだ削除しません）", exact: true });
  const modal = () => page.getByRole("dialog", { name: "モデル削除の対象確認（読み取り専用）", exact: true });
  const boot = async () => { await page.reload(); await trigger().waitFor(); await page.waitForFunction(() => !document.body.textContent.includes("配置とプロファイル構成を再読込しています。")); };
  const fixture = patch => page.evaluate(p => window.modelDeletionFixture.update(p), patch);
  const review = async () => { await trigger().click(); await modal().waitFor(); };
  const closed = () => modal().waitFor({ state: "hidden" });
  const onlyReadCommands = async () => check(await page.evaluate(() => window.modelDeletionFixture.calls.every(c => ["get_model_placements", "get_model_profiles", "get_model_download_catalog", "preview_model_deletion", "cancel_model_inspection"].includes(c.command))), "mutation or execution command invoked");
  await boot(); await review();
  const body = await modal().innerText();
  check(body.includes("欠落 1 件") && body.includes("登録内容から変更 1 件") && body.includes("未登録ファイル 1 件"), "classifications hidden");
  check(body.includes("アプリ全体で 1 件") && body.includes("現在の利用中プロセス数や削除可否を示しません"), "reference counts presented as permission");
  check(await modal().getByRole("button").count() === 4, "unexpected action exposed");
  check(await modal().getByRole("button").evaluateAll(buttons => buttons.every(button => button.hasAttribute("data-dialog-cancel") || ["前の50件", "次の50件"].includes(button.textContent))), "execution action exposed");
  check(await modal().getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "safe initial cancellation focus missing");
  await page.getByRole("button", { name: "削除せず閉じる", exact: true }).click(); await closed();
  check(await trigger().evaluate(e => e === document.activeElement), "opener focus not restored");
  await onlyReadCommands(); results.push("read-only classifications and historical counts; explicit close and focus");

  await review(); await page.keyboard.press("Escape"); await closed();
  check(await trigger().evaluate(e => e === document.activeElement), "Escape did not restore focus");
  results.push("Escape closes without mutation");
  await fixture({ mode: "many" }); await review();
  check(await modal().getByRole("listitem").count() === 50, "first page size wrong");
  await modal().getByRole("button", { name: "次の50件", exact: true }).click();
  check((await modal().innerText()).includes("一覧 51–100 / 121 件"), "second page missing");
  await modal().getByRole("button", { name: "次の50件", exact: true }).click();
  check(await modal().getByRole("listitem").count() === 21 && (await modal().innerText()).includes("file-119日本語"), "tail silently omitted");
  check(await modal().getByRole("button", { name: "次の50件", exact: true }).isDisabled(), "last page allowed overflow");
  await modal().getByRole("button", { name: "前の50件", exact: true }).click();
  check((await modal().innerText()).includes("一覧 51–100 / 121 件"), "previous page wrong");
  await page.keyboard.press("Escape"); await review();
  check((await modal().innerText()).includes("一覧 1–50 / 121 件"), "new review did not reset page");
  await page.keyboard.press("Escape"); await closed();
  results.push("121 objects: full pagination, tail, boundary and fresh-review reset");

  for (const mode of ["forged", "wrongPackage", "wrongToken", "wrongTotal"]) {
    await boot(); await fixture({ mode }); await trigger().click();
    await page.getByRole("alert").filter({ hasText: "対象検査と選択した配置・操作が一致しません" }).waitFor();
    check(await modal().count() === 0, "invalid reply opened a modal");
    await onlyReadCommands(); results.push(`${mode}: invalid response rejected`);
  }
  await boot(); await fixture({ mode: "failure" }); await trigger().click();
  await page.getByRole("alert").filter({ hasText: "模擬リンク検出" }).waitFor();
  check(await modal().count() === 0, "failed native review retained a modal");
  await fixture({ mode: "success" }); await review(); await page.keyboard.press("Escape"); await closed();
  results.push("native inspection failure: preserve, explain and explicitly retry");

  await boot(); await fixture({ mode: "delay" }); await trigger().click();
  const cancel = page.getByRole("button", { name: "削除対象の検査を取り消す", exact: true });
  await cancel.waitFor(); check(await trigger().isDisabled(), "parallel review enabled");
  await cancel.click(); check(await cancel.isDisabled(), "duplicate cancellation enabled");
  await page.evaluate(() => window.modelDeletionFixture.complete());
  await page.waitForFunction(() => !document.body.textContent.includes("取消要求済み。"));
  check(await modal().count() === 0, "cancelled late response resurrected review");
  const calls = await page.evaluate(() => window.modelDeletionFixture.calls);
  check(calls.find(c => c.command === "preview_model_deletion").args.token === calls.find(c => c.command === "cancel_model_inspection").args.token, "cancel targeted another operation");
  await onlyReadCommands(); results.push("cancelled late response: no resurrection and same operation identity");

  await boot(); await fixture({ partial: true, mode: "absent" });
  await page.getByRole("button", { name: "配置・構成記録を再読込", exact: true }).click();
  await page.getByText("未完了の配置・途中ファイルは保全", { exact: false }).waitFor();
  await review(); check((await modal().innerText()).includes("0 bytes") && (await modal().innerText()).includes("欠落 1 件"), "partial absent package hidden or relabeled ready");
  await page.keyboard.press("Escape"); await closed();
  results.push("partial/absent placement remains inspectable without readiness");

  for (const flag of ["recordingActive", "locked"]) {
    await boot(); await review(); await fixture({ [flag]: true }); await closed();
    check(await trigger().isDisabled(), "new review allowed during state gate");
    await page.getByRole("alert").filter({ hasText: "録音・操作の状態が変わった" }).waitFor();
    await onlyReadCommands(); results.push(`${flag}: close stale review and prevent new inspection`);
  }
  await boot(); await fixture({ mode: "delay" }); await trigger().click();
  await page.getByRole("button", { name: "削除対象の検査を取り消す", exact: true }).waitFor();
  await fixture({ mounted: false });
  await page.waitForFunction(() => window.modelDeletionFixture.calls.some(c => c.command === "cancel_model_inspection"));
  await page.evaluate(() => window.modelDeletionFixture.complete());
  check(await modal().count() === 0, "unmounted response opened review");
  await onlyReadCommands(); results.push("unmount cancels inspection and suppresses late result");
  await boot(); await fixture({ failCatalog: true });
  await page.getByRole("button", { name: "配置・構成記録を再読込", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "模擬配置読込失敗" }).waitFor();
  // Stale catalog must not provide a target after a failed refresh.
  check(await trigger().isDisabled(), "failed refresh allowed stale target review");
  results.push("catalog refresh failure rejects stale deletion target");
  return { passed: results.length, results };
}
