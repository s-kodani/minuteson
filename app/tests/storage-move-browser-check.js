// CLI-first mocked IPC verification; no live recording or filesystem mutation.
async (page) => {
  const results = [];
  const check = (value, message) => { if (!value) throw Error(message); };
  const region = () => page.getByRole("region", { name: "データと保存" });
  const dialog = () => page.getByRole("dialog");
  const set = values => page.evaluate(v => Object.assign(window.dataFixture, v), values);
  const count = () => page.evaluate(() => window.dataFixture.calls.filter(c => c.command === "start_data_operation").length);
  const boot = async () => {
    await page.reload(); await page.evaluate(() => { window.confirm = () => true; });
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await region().getByRole("button", { name: "会議保存先を変更", exact: true }).waitFor();
  };
  const choose = async () => { await region().getByRole("button", { name: "会議保存先を変更", exact: true }).click(); await dialog().waitFor(); };
  const publish = () => dialog().getByRole("button", { name: "コピーして保存先を切替", exact: true }).click();
  const finished = () => region().getByRole("alert").filter({ hasText: "保存先切替の書込停止中です" }).waitFor();

  await boot(); await choose();
  const body = await dialog().innerText();
  check(body.includes("/safe/root") && body.includes("/safe/chosen/MON-meetings-new") && body.includes("会議 3 件・ファイル 8 件") && body.includes("モデルと起動設定は移動しません"), "move review lacks roots/count/scope");
  check(await count() === 0, "move published before explicit confirmation");
  await page.screenshot({ path: "output/playwright/storage-move-review.png" });
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "会議保存先を変更");
  check(await count() === 0, "Escape published move");
  results.push("review roots/count/scope, no auto-publication, Escape and focus");

  await boot(); await set({ previewMode: "capacity" }); await choose();
  check(await dialog().getByRole("button", { name: "コピーして保存先を切替", exact: true }).isDisabled(), "capacity shortage admitted");
  const before = await page.evaluate(() => window.dataFixture.snapshot.relocationPreview);
  await dialog().getByRole("button", { name: "同じ保存先を再確認", exact: true }).click(); await dialog().waitFor();
  const after = await page.evaluate(() => window.dataFixture.snapshot.relocationPreview);
  check(before.operationId === after.operationId && before.confirmationId !== after.confirmationId, "refresh lost operation or retained stale confirmation");
  await publish(); await finished();
  check(await region().getByRole("button", { name: "バックアップを作成", exact: true }).isDisabled(), "backup enabled after retirement");
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "editing enabled after retirement");
  check((await region().innerText()).includes("/safe/app-data/models"), "model root lost");
  check(await page.evaluate(() => window.dataFixture.published) === 1, "move duplicated");
  await page.screenshot({ path: "output/playwright/storage-move-restart.png" });
  results.push("capacity refusal, refreshed same operation, single cutover and restart write lock");

  await boot(); await set({ mode: "lostDelay", suppressEvents: true }); await choose(); await publish();
  await page.waitForFunction(() => window.dataFixture.snapshot.phase === "copying");
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "lost RPC unlocked running copy");
  await page.evaluate(() => window.dataFixture.complete()); await finished();
  check(await page.evaluate(() => window.dataFixture.published) === 1 && await count() === 1, "lost response resubmitted or duplicated");
  results.push("lost response and suppressed events recover through snapshot without duplicate");

  await boot(); await set({ mode: "delay" }); await choose(); await publish();
  await region().getByRole("button", { name: "処理を取り消す", exact: true }).click();
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "cancellation unlocked running copy");
  await page.evaluate(() => window.dataFixture.complete());
  await region().getByRole("alert").filter({ hasText: "未完了の保存先変更があります" }).waitFor();
  check(await region().getByRole("button", { name: "会議保存先を変更", exact: true }).isDisabled(), "unresolved copy admitted new move");
  check(await page.evaluate(() => window.dataFixture.published) === 0, "cancel selected partial target");
  check(!(await region().innerText()).includes("保存先を切り替えました。"), "partial copy claimed completed");
  results.push("cancel waits for actual completion, keeps old selection and pending copy");

  await boot(); await set({ mode: "stale" }); await choose(); await publish();
  await region().getByRole("alert").filter({ hasText: "確認後に会議・編集・音声が変更" }).waitFor();
  await region().getByRole("button", { name: "確認内容を開く", exact: true }).click();
  check(await dialog().getByRole("button", { name: "コピーして保存先を切替", exact: true }).isDisabled(), "failed preview reusable without refresh");
  await set({ mode: "success" }); await dialog().getByRole("button", { name: "同じ保存先を再確認", exact: true }).click(); await dialog().waitFor(); await publish(); await finished();
  results.push("changed content refuses stale confirmation, refresh admits new confirmation");

  await boot(); await set({ mode: "cancelRace" }); await choose(); await publish();
  await region().getByRole("button", { name: "処理を取り消す", exact: true }).click();
  await page.evaluate(() => window.dataFixture.complete()); await finished();
  check(await page.evaluate(() => window.dataFixture.published) === 1, "late cancel erased committed cutover");
  results.push("durable selection wins over late cancellation");

  await boot(); await set({ mode: "forged" }); await choose(); await publish();
  await region().getByRole("alert").filter({ hasText: "状態が不正" }).waitFor();
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "forged receipt unlocked writes");
  check(await page.evaluate(() => window.dataFixture.published) === 0, "forged receipt published");
  results.push("foreign receipt rejected, unknown IPC state locks writes");
  return results;
}
