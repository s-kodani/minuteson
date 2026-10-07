// CLI-first browser checks with local mocked IPC, not native WebView proof.
// Use the data-save-browser-fixture.js index route and playwright-cli run-code.
async (page) => {
  const results = [];
  const check = (value, message) => { if (!value) throw Error(message); };
  const set = values => page.evaluate(v => Object.assign(window.dataFixture, v), values);
  const calls = command => page.evaluate(c => window.dataFixture.calls.filter(x => x.command === c), command);
  const region = () => page.getByRole("region", { name: "中断したデータ操作の確認" });
  const dialog = () => page.getByRole("dialog");
  const boot = async (source = "scratch", options = {}) => {
    await page.reload();
    await page.waitForFunction(() => !!window.dataFixture);
    await page.evaluate(({ source, options }) => {
      const id = crypto.randomUUID();
      window.dataFixture.recoveryItems = [{ source, operationId: id, paths: [source === "scratch" ? `.mon-stage-${id}` : `meetings/${crypto.randomUUID()}`], registeredPaths: 2, reason: "所有不明のファイルを保持しています", retained: false }];
      Object.assign(window.dataFixture, options);
    }, { source, options });
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await region().waitFor();
    if (!options.recoveryReadFail && !options.recoveryUnsupported) await region().getByRole("button", { name: "対象と解決方法を確認", exact: true }).waitFor();
  };
  const review = async () => {
    await region().getByRole("button", { name: "対象と解決方法を確認", exact: true }).click();
    await dialog().getByRole("heading", { name: "未確定データの解決方法を確認", exact: true }).waitFor();
  };
  const act = action => dialog().getByRole("button", { name: action === "keep" ? "保持して確認を完了" : "再検査して回収", exact: true }).click();
  const retained = () => region().getByRole("status").filter({ hasText: "回収はしていません" }).waitFor();

  await boot(); await review();
  check((await calls("start_data_operation")).length === 0, "readonly review mutated files");
  check((await dialog().innerText()).includes("元バックアップ") === false && (await region().innerText()).includes("/safe/root"), "target root absent");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "対象と解決方法を確認");
  check((await calls("start_data_operation")).length === 0, "Escape resolved work");
  results.push("readonly inspection, Escape cancellation, focus return");

  for (const source of ["scratch", "restore_addition"]) {
    await boot(source); await review(); await act("keep"); await retained();
    await region().getByRole("status").filter({ hasText: "未確認 0 件 · 保持して確認済み 1 件" }).waitFor();
    await page.waitForFunction(() => document.activeElement?.textContent === "中断したデータ操作の確認");
    check(await region().getByRole("button", { name: "対象と解決方法を確認", exact: true }).count() === 0, "kept target still actionable");
    check(!(await region().innerText()).includes("対象を回収しました"), "keep falsely called reclaimed");
    check((await calls("start_data_operation")).length === 1 && await page.evaluate(() => window.dataFixture.meetings.length) === 1, "keep duplicated or replaced existing");
    results.push(`${source}: explicit keep, not recovered, no repeat action`);
  }

  await boot(); await review(); await act("recover");
  await region().getByRole("status").filter({ hasText: "記録済みの対象を回収しました" }).waitFor();
  await region().getByRole("status").filter({ hasText: "未確認 0 件" }).waitFor();
  results.push("confirmed recovery removes only mocked owned target");

  await boot("restore_addition", { recoveryMode: "attention" }); await review(); await act("recover");
  await region().getByRole("alert").filter({ hasText: "要確認が残っています" }).waitFor();
  check((await region().innerText()).includes("未確認 1 件") && !(await region().innerText()).includes("対象を回収しました"), "remaining attention called success");
  results.push("remaining unknown data is attention, not recovered");

  await boot("scratch", { recoveryMode: "stale" }); await review();
  const previous = await page.evaluate(() => structuredClone(window.dataFixture.snapshot.recoveryPreview));
  await act("keep"); await dialog().waitFor({ state: "hidden" });
  await region().getByRole("alert").filter({ hasText: "対象が変更" }).waitFor();
  await region().getByRole("button", { name: "回収の確認内容を開く", exact: true }).click();
  check(await dialog().getByRole("button", { name: "保持して確認を完了", exact: true }).isDisabled(), "stale preview admitted keep");
  await dialog().getByRole("button", { name: "最新の対象を再確認", exact: true }).click();
  await dialog().waitFor();
  const fresh = await page.evaluate(() => window.dataFixture.snapshot.recoveryPreview);
  check(previous.item.operationId === fresh.item.operationId && previous.confirmationId !== fresh.confirmationId, "refresh changed operation or reused consent");
  await act("keep"); await retained();
  results.push("stale metadata blocks mutation, refresh replaces confirmation only");

  for (const mode of ["delay", "cancelRace"]) {
    await boot("scratch", { recoveryMode: mode }); await review(); await act("keep");
    await region().getByRole("button", { name: "安全な停止を要求", exact: true }).click();
    const stopChoice = page.getByRole("dialog", { name: "回収操作の安全な停止を確認", exact: true });
    await stopChoice.getByRole("button", { name: "安全な停止を要求して待つ", exact: true }).click();
    check(await page.getByRole("button", { name: "概要", exact: true }).isDisabled() && await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "cancellation unlocked before worker end");
    await page.evaluate(() => window.dataFixture.complete()); await stopChoice.waitFor({ state: "hidden" });
    if (mode === "cancelRace") await retained();
    else {
      await page.waitForFunction(() => window.dataFixture.snapshot.phase === "cancelled");
      check(await page.evaluate(() => window.dataFixture.recoveryItems[0].retained) === false, "cancel retained target");
    }
    results.push(mode === "cancelRace" ? "late cancellation cannot erase keep receipt" : "cancellation waits for actual worker, no keep");
  }

  await boot("scratch", { recoveryMode: "lost", suppressEvents: true }); await review(); await act("keep"); await retained();
  check((await calls("start_data_operation")).length === 1, "lost accepted reply duplicated action");
  results.push("lost reply and events recovered by polling without another admission");

  await boot("scratch", { recoveryMode: "lostNoCommit" }); await review(); await act("keep");
  await dialog().getByRole("alert").filter({ hasText: "受付結果が未確定" }).waitFor();
  check(await dialog().getByRole("button", { name: "再検査して回収", exact: true }).isDisabled(), "unknown reply allowed different action");
  await set({ recoveryMode: "success" });
  await dialog().getByRole("button", { name: "同じ確認操作の結果を確認", exact: true }).click(); await retained();
  const attempts = await calls("start_data_operation");
  check(attempts.length === 2 && JSON.stringify(attempts[0].args) === JSON.stringify(attempts[1].args), "retry did not reuse exact request");
  results.push("unknown unaccepted request: reachable modal retry retains exact identity");

  await boot("scratch", { recoveryMode: "lostNoCommit" }); await review(); await act("keep");
  await dialog().getByRole("button", { name: "同じ確認操作を取り消す", exact: true }).click();
  await dialog().waitFor({ state: "hidden" });
  check((await calls("start_data_operation")).length === 1 && await page.evaluate(() => window.dataFixture.recoveryItems[0].retained) === false, "unknown cancellation kept or repeated work");
  results.push("unknown request: explicit native cancellation without mutation");

  await boot("scratch", { recoverySelectLost: true, suppressEvents: true }); await review();
  check((await calls("review_recovery_item")).length === 1 && (await calls("start_data_operation")).length === 0, "selection response loss repeated inspection or resolved");
  await act("keep"); await retained();
  results.push("lost inspection reply: readonly polling recovers explicit review");

  await boot("scratch", { recoveryReadFail: true });
  await region().getByRole("alert").filter({ hasText: "回収一覧失敗" }).waitFor();
  check(await region().getByRole("button", { name: "対象と解決方法を確認", exact: true }).count() === 0, "failed listing exposes mutation");
  await set({ recoveryReadFail: false }); await region().getByRole("button", { name: "要確認一覧を再読込", exact: true }).click(); await review();
  await dialog().getByRole("button", { name: "取り消して閉じる", exact: true }).click();
  results.push("listing read failure fails closed and manual read recovers");

  await boot("scratch", { recoveryUnsupported: true, recoveryItems: [] });
  await region().getByRole("status").filter({ hasText: "このOSの回収操作は保留中" }).waitFor();
  check(await region().getByRole("button", { name: "対象と解決方法を確認", exact: true }).count() === 0, "deferred platform exposed actions");
  results.push("deferred platform has no recovery actions");

  await boot(); await set({ failSave: true });
  await page.getByRole("button", { name: "用語を追加", exact: true }).click();
  await page.getByRole("textbox", { name: "用語（必須）", exact: true }).fill("点検日");
  await region().getByRole("button", { name: "対象と解決方法を確認", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "編集内容はこの画面に保持" }).waitFor();
  check((await calls("review_recovery_item")).length === 0 && await page.getByRole("textbox", { name: "用語（必須）", exact: true }).inputValue() === "点検日", "failed save admitted review or lost input");
  results.push("draft save failure blocks inspection and preserves input");

  await boot("scratch", { recoveryMode: "forged" }); await review(); await act("keep");
  await dialog().getByRole("alert").filter({ hasText: "状態が不正" }).waitFor();
  check(await page.getByRole("button", { name: "概要", exact: true }).isDisabled() && !(await region().innerText()).includes("回収はしていません"), "forged receipt unlocked or displayed success");
  results.push("wrong-operation receipt rejected, no success, protection remains");
  console.log(JSON.stringify({ passed: results.length, results }));
  return { passed: results.length, results };
}
