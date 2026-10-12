// Synthetic IPC only; never opens or deletes the user's native meeting data.
async (page, capturePath) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const absent = async () => {
    check(await page.getByRole("button", { name: "中断・削除の管理", exact: true }).count() === 0, "obsolete management entry remains");
    check(await page.locator(".recovery-panel, .partial-recording-panel, .deletion-panel, .settings-management-toggle").count() === 0, "administrative panel remains mounted");
    check(await page.evaluate(() => window.dataFixture.calls.every(call => !["get_recovery_items", "get_partial_recording_items", "get_partial_recording_results", "review_recovery_item", "review_partial_recording"].includes(call.command))), "removed manual recovery IPC was called");
  };
  await page.getByRole("complementary").filter({ has: page.getByRole("button", { name: "設定", exact: true }) }).getByRole("button", { name: "設定", exact: true }).click();
  await page.getByRole("button", { name: "バックアップを作成", exact: true }).waitFor();
  await absent();
  await page.screenshot({ path: capturePath("settings-without-maintenance.png") });
  results.push("settings contain no manual interrupted/deleted data management or recovery calls");
  await page.reload();
  await page.getByRole("button", { name: "既存会議の操作", exact: true }).waitFor();
  await absent();
  await page.getByRole("button", { name: "既存会議の操作", exact: true }).click();
  check(await page.getByRole("menuitem", { name: "開く", exact: true }).isVisible(), "normal meeting menu disappeared");
  check(await page.getByRole("menuitem", { name: "既存会議を削除", exact: true }).isVisible(), "normal deletion disappeared");
  await page.keyboard.press("Escape");
  results.push("ordinary meeting menu remains while administrative panels stay absent");
  return { passed: results.length, results };
}
