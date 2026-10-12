// Production data panels with data-save-browser-fixture.js; no real files.
async page => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const data = () => page.getByRole("region", { name: "データと保存", exact: true });
  const recovery = () => page.getByRole("region", { name: "中断したデータ操作の確認", exact: true });
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const count = command => page.evaluate(c => window.dataFixture.calls.filter(x => x.command === c).length, command);
  const boot = async () => { await page.reload(); await page.getByRole("button", { name: "設定", exact: true }).click(); await data().waitFor(); };
  const moving = () => page.getByRole("dialog", { name: /^(会議保存先の変更確認|未完了移動の確認)$/u });
  const chooseMove = async () => { await data().getByRole("button", { name: "会議保存先を変更", exact: true }).click(); await moving().waitFor(); };
  const stopTitle = "データ操作の安全な取消を確認", changeTitle = "データ操作の確認内容変更を確認";
  const changeButton = () => data().getByRole("button", { name: "バックアップから追加復元", exact: true });
  const failedMove = async () => { await boot(); await chooseMove(); await page.evaluate(() => window.dataFixture.mode = "stale"); await moving().getByRole("button", { name: "コピーして保存先を切替", exact: true }).click(); await moving().waitFor({ state: "hidden" }); await page.getByRole("complementary", { name: "通知", exact: true }).getByRole("alert").filter({ hasText: "確認後に" }).waitFor(); };
  await failedMove(); const old = await page.evaluate(() => JSON.stringify(window.dataFixture.snapshot));
  await changeButton().click(); await modal(changeTitle).waitFor();
  check(await count("select_data_operation") === 1 && (await modal(changeTitle).innerText()).includes("元バックアップは変更しません"), "reselect ran before decision or omitted scope");
  await page.keyboard.press("Escape"); await modal(changeTitle).waitFor({ state: "hidden" });
  check(await page.evaluate(() => JSON.stringify(window.dataFixture.snapshot)) === old && await count("select_data_operation") === 1, "cancel changed native review");
  await changeButton().click(); await modal(changeTitle).getByRole("button", { name: "確認内容を変更して選び直す", exact: true }).click(); await modal(changeTitle).waitFor({ state: "hidden" });
  await modal("バックアップから追加復元").waitFor();
  check(await count("select_data_operation") === 2 && await count("start_data_operation") === 1, "explicit selection duplicated or published restore");
  results.push("changing failed review requires consent; cancel retains exact snapshot, one accepted selection opens a separate restore review");

  await failedMove(); await changeButton().click(); await modal(changeTitle).waitFor();
  await page.evaluate(() => window.dataFixture.changed({ errorMessage: "確認内容を更新" }));
  await modal(changeTitle).getByRole("button", { name: "確認内容を変更して選び直す", exact: true }).click(); await modal(changeTitle).getByRole("alert").waitFor();
  check(await count("select_data_operation") === 1, "changed review accepted stale consent");
  await modal(changeTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  results.push("changed native review invalidates choice before selection without retry");

  await failedMove(); await page.getByRole("button", { name: "用語を追加", exact: true }).click();
  await page.getByRole("textbox", { name: "用語（必須）", exact: true }).fill("点検日");
  await changeButton().click(); await modal(changeTitle).waitFor(); await page.evaluate(() => window.dataFixture.failSave = true);
  await modal(changeTitle).getByRole("button", { name: "確認内容を変更して選び直す", exact: true }).click(); await modal(changeTitle).getByRole("alert").waitFor();
  check(await count("select_data_operation") === 1 && await page.getByRole("textbox", { name: "用語（必須）", exact: true }).inputValue() === "点検日", "failed save selected files or lost local input");
  await modal(changeTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  results.push("swallowed navigation save failure is retained by confirmation; no selection or false completion");

  await boot(); await chooseMove(); await page.evaluate(() => window.dataFixture.mode = "delay");
  await moving().getByRole("button", { name: "コピーして保存先を切替", exact: true }).click();
  const token = await page.evaluate(() => window.dataFixture.snapshot.token);
  const stopButton = data().getByRole("button", { name: "処理を取り消す", exact: true });
  await stopButton.click(); await modal(stopTitle).waitFor(); await page.keyboard.press("Escape"); await modal(stopTitle).waitFor({ state: "hidden" });
  check(await count("cancel_data_operation") === 0 && await page.evaluate(() => window.dataFixture.snapshot.phase === "copying"), "declined cancellation stopped copy");
  await stopButton.click(); await modal(stopTitle).getByRole("button", { name: "取消を要求して停止を待つ", exact: true }).dblclick();
  await page.waitForFunction(() => window.dataFixture.snapshot.phase === "cancelling");
  for (let i = 0; i < 5; i++) await page.keyboard.press("Escape");
  check(await modal(stopTitle).isVisible() && await count("cancel_data_operation") === 1 && await page.getByRole("navigation", { name: "メインナビゲーション", exact: true }).getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "stop repeated or unlocked before worker termination");
  await page.evaluate(() => window.dataFixture.complete()); await modal(stopTitle).waitFor({ state: "hidden" });
  check(await page.evaluate(t => window.dataFixture.calls.find(c => c.command === "cancel_data_operation").args.token === t && window.dataFixture.published === 0 && window.dataFixture.pendingMove !== null, token), "stop changed token or discarded partial copy");
  results.push("one explicit same-token stop waits for actual worker through repeated Escape and preserves partial copy");
  await data().getByRole("button", { name: "未完了移動を確認", exact: true }).click(); await modal(changeTitle).waitFor();
  check((await modal(changeTitle).innerText()).includes("確認だけでは切替・削除しません"), "pending inspection warning absent");
  await page.keyboard.press("Escape"); await modal(changeTitle).waitFor({ state: "hidden" }); check(await count("review_storage_move") === 0, "cancel inspected pending move");
  await data().getByRole("button", { name: "未完了移動を確認", exact: true }).click(); await modal(changeTitle).getByRole("button", { name: "未完了移動を再確認する", exact: true }).click(); await moving().waitFor();
  check(await count("review_storage_move") === 1 && await page.evaluate(() => window.dataFixture.published === 0), "inspection executed cutover or repeated");
  results.push("attention and uncompleted move inspection require distinct review consent without selecting or cutting over");

  await boot(); await chooseMove(); await page.evaluate(() => { window.dataFixture.mode = "delay"; window.dataFixture.cancelLost = true; });
  await moving().getByRole("button", { name: "コピーして保存先を切替", exact: true }).click();
  await data().getByRole("button", { name: "処理を取り消す", exact: true }).click(); await modal(stopTitle).getByRole("button", { name: "取消を要求して停止を待つ", exact: true }).click(); await modal(stopTitle).getByRole("alert").waitFor();
  check(await count("cancel_data_operation") === 1 && await modal(stopTitle).getByRole("button", { name: "取消を要求して停止を待つ", exact: true }).isDisabled(), "lost stop acknowledgement repeated cancellation");
  await modal(stopTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  check(await page.getByRole("navigation", { name: "メインナビゲーション", exact: true }).getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "uncertain stop removed native write fence");
  await page.evaluate(() => window.dataFixture.complete());
  results.push("lost stop acknowledgement stays visible without replay and leaves native write fence until worker terminates");

  await boot(); await chooseMove(); await moving().getByRole("button", { name: "コピーして保存先を切替", exact: true }).click();
  const exit = data().getByRole("button", { name: "終了する（再起動は手動）", exact: true }); await exit.waitFor(); await exit.click();
  const exitTitle = "保存先切替後の終了を確認"; await modal(exitTitle).waitFor(); await page.keyboard.press("Escape"); await modal(exitTitle).waitFor({ state: "hidden" }); check(await count("finish_main_close") === 0, "cancel quit app");
  await exit.click(); await page.evaluate(() => window.dataFixture.failClose = true); await modal(exitTitle).getByRole("button", { name: "安全に終了する", exact: true }).click(); await modal(exitTitle).getByRole("alert").waitFor();
  check(await count("finish_main_close") === 1 && (await modal(exitTitle).innerText()).includes("手動で再起動") && await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "exit failure replayed or lost retirement fence");
  await modal(exitTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  results.push("manual restart exit requires consent and retains failed exit plus retirement write protection without retry");
  return { passed: results.length, results };
}
