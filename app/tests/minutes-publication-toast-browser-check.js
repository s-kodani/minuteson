async (page, capturePath, detailUI) => {
  const results = [], check = (v, message) => { if (!v) throw Error(message); };
  const panel = () => page.getByRole("region", { name: "議事録生成と候補確認", exact: true });
  const host = () => page.getByRole("complementary", { name: "通知", exact: true });
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const count = cmd => page.evaluate(c => window.__mon.calls.filter(x => x.cmd === c).length, cmd);
  const boot = async () => { await page.reload(); await page.locator(".meeting-name").first().click(); await detailUI.openMinutes(); await panel().getByRole("button", { name: "議事録直前版を確認・復元", exact: true }).waitFor(); };
  const restore = async () => { await panel().getByRole("button", { name: "議事録直前版を確認・復元", exact: true }).click(); await modal("議事録直前版の復元確認 #1").getByRole("button", { name: "この議事録直前版へ戻す", exact: true }).click(); await modal("議事録の直前版復元を確認").getByRole("button", { name: "直前版へ戻す", exact: true }).click(); };
  await boot(); check(await host().getByRole("status").filter({ hasText: "元データが変更" }).count() === 0, "historical stale minutes generated toast");
  const original = await page.evaluate(() => JSON.stringify(window.__mon.drafts.a));
  await restore(); await modal("議事録直前版の復元確認 #1").waitFor({ state: "hidden" });
  await host().getByRole("status").filter({ hasText: "議事録の直前版へ戻しました" }).waitFor();
  check(await count("restore_minutes_version") === 1 && await page.evaluate(() => window.__mon.current === "p" && window.__mon.previous === "a" && JSON.stringify(window.__mon.drafts.a)) === original, "restore discarded draft or repeated");
  results.push("historical source-update state is quiet; verified restore notifies once and preserves the old draft");

  for (const mode of ["lostPublicationResponse", "foreignPublicationReceipt"]) {
    await boot(); await page.evaluate(mode => window.__mon[mode] = true, mode); await restore();
    await modal("議事録の直前版復元を確認").getByRole("alert").waitFor(); await page.keyboard.press("Escape"); await modal("議事録直前版の復元確認 #1").waitFor();
    check(await host().getByRole("status").filter({ hasText: "直前版へ戻しました" }).count() === 0, "unverified restore announced success");
    await page.keyboard.press("Escape"); await panel().getByRole("button", { name: "採用・復元の結果だけを確認", exact: true }).click();
    await host().getByRole("status").filter({ hasText: "議事録の直前版へ戻しました" }).waitFor();
    check(await count("restore_minutes_version") === 1 && await page.evaluate(() => window.__mon.current === "p" && window.__mon.previous === "a"), "result check inverted restore");
    results.push(`${mode}: reject false restore success and verify current head without issuing a second restore`);
  }

  await boot(); await panel().getByRole("button", { name: "議事録を再生成", exact: true }).click(); await page.waitForFunction(() => window.__mon.jobs.length === 1);
  await page.evaluate(() => window.__mon.complete()); await panel().getByRole("button", { name: "議事録候補を確認", exact: true }).click();
  await modal("議事録候補の採用確認 #3").getByRole("button", { name: "この議事録候補を採用", exact: true }).click(); await modal("議事録候補の採用を確認").getByRole("button", { name: "候補を採用する", exact: true }).click();
  await modal("議事録候補の採用確認 #3").waitFor({ state: "hidden" }); await host().getByRole("status").filter({ hasText: "議事録候補を採用しました" }).waitFor();
  check(await count("adopt_minutes_candidate") === 1 && await page.evaluate(() => window.__mon.current === "b" && window.__mon.previous === "a" && !!window.__mon.drafts.a), "adopted wrong minutes or lost old edit");
  results.push("verified minutes adoption identifies its own target and retains the prior draft");

  await boot(); await page.evaluate(() => { window.__mon.versions.find(v => v.id === "a").inputUpdated = false; });
  await panel().getByRole("button", { name: "議事録を再読込", exact: true }).click(); await panel().getByText("元データ更新あり", { exact: true }).waitFor({ state: "hidden" });
  const before = await page.evaluate(() => JSON.stringify({ views: window.__mon.views, drafts: window.__mon.drafts }));
  await page.evaluate(() => { window.__mon.versions.find(v => v.id === "a").inputUpdated = true; });
  await panel().getByRole("button", { name: "議事録を再読込", exact: true }).click();
  const updated = host().getByRole("status").filter({ hasText: "議事録作成後に元データが変更されています" }); await updated.waitFor();
  const id = await updated.locator("xpath=ancestor::section[contains(@class,'toast-card')]").getAttribute("data-toast-id");
  await panel().getByRole("button", { name: "議事録を再読込", exact: true }).click();
  check(await updated.locator("xpath=ancestor::section[contains(@class,'toast-card')]").getAttribute("data-toast-id") === id && await count("start_minutes_generation") === 0 && await page.evaluate(() => JSON.stringify({ views: window.__mon.views, drafts: window.__mon.drafts })) === before, "polling re-notified, regenerated or replaced fixed evidence");
  await host().getByRole("button", { name: "注意通知を閉じる", exact: true }).click();
  check(await panel().getByText("元データ更新あり", { exact: true }).isVisible(), "dismissal erased source-update state");
  await panel().getByRole("button", { name: "議事録の元データ更新の説明を開く", exact: true }).click();
  await page.getByRole("dialog", { name: "議事録の元データ更新", exact: true }).waitFor();
  results.push("fresh source update notifies once; dismissal retains status, fixed evidence and explicit regeneration only");
  for (const theme of ["light", "dark"]) { await page.emulateMedia({ colorScheme: theme }); await page.waitForFunction(t => document.documentElement.dataset.theme === t, theme); await page.screenshot({ path: `minutes-source-update-${theme}.png` }); }
  await page.emulateMedia({ colorScheme: null }); await page.keyboard.press("Escape");
  await boot(); await panel().getByText("議事録エンジン設定（端末内に保存）", { exact: true }).click();
  const threads = panel().getByRole("spinbutton", { name: "議事録CPUスレッド数", exact: true });
  await page.evaluate(() => window.__mon.failPreferences = true); await threads.fill("6");
  await host().getByRole("alert").filter({ hasText: "議事録設定を保存できません" }).waitFor();
  const saves = await count("save_minutes_preferences"); await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await threads.inputValue() === "6" && await count("save_minutes_preferences") === saves && await panel().getByLabel("議事録設定の保存の要対応状態", { exact: true }).isVisible(), "minutes settings dismissal wrote or lost input");
  await page.evaluate(() => window.__mon.failPreferences = false); await panel().getByRole("button", { name: "議事録設定保存を再試行", exact: true }).click();
  await panel().getByLabel("議事録設定の保存の要対応状態", { exact: true }).waitFor({ state: "hidden" });
  results.push("minutes settings save failure retains input and issue after dismissal, then resolves through explicit queue retry");
  await boot(); await page.evaluate(() => window.__mon.failStart = true); await panel().getByRole("button", { name: "議事録を再生成", exact: true }).click();
  await host().getByRole("alert").filter({ hasText: "議事録操作を完了できません" }).waitFor(); await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await panel().getByLabel("議事録操作の要対応状態", { exact: true }).isVisible() && await count("start_minutes_generation") === 1 && await page.evaluate(() => window.__mon.current === "a" && window.__mon.jobs.length === 0), "minutes start error was swallowed, retried or replaced the head");
  results.push("minutes command failure crosses the saved-edit guard and survives dismissal without execution retry or document replacement");
  return { passed: results.length, results };
}
