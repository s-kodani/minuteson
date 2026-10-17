async page => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.goto("http://127.0.0.1:1421/tests/detail-mock-browser.html?view=minutes");
  const panel = () => page.getByRole("region", { name: "議事録", exact: true });
  const open = async () => {
    await page.getByRole("button", { name: "議事録UI確認", exact: true }).click();
    await page.getByRole("tab", { name: "議事録", exact: true }).click();
    await panel().getByRole("button", { name: "議事録を生成", exact: true }).waitFor();
  };
  await open();
  await page.evaluate(() => { window.__mon.startMode = "held"; });
  await panel().getByRole("button", { name: "議事録を生成", exact: true }).click();
  const dialog = page.locator("dialog[open]");
  await dialog.getByRole("button", { name: "議事録を再生成", exact: true }).click();
  await page.waitForFunction(() => typeof window.__mon.startRelease === "function");
  await dialog.waitFor({ state: "hidden" });
  check(await page.evaluate(() => window.__mon.jobs.length === 0), "preparation was represented as an accepted job");
  await page.getByRole("region", { name: "進行中の処理" }).getByText("議事録生成 · 開始準備中", { exact: true }).waitFor();
  const sidebar = page.locator(".sidebar");
  check(await sidebar.getByRole("button", { name: "設定", exact: true }).isEnabled(), "settings remain globally locked");
  await sidebar.getByRole("button", { name: "会議一覧", exact: true }).click();
  await page.getByRole("button", { name: "議事録UI確認", exact: true }).waitFor();
  await page.getByRole("region", { name: "進行中の処理" }).getByText("議事録生成 · 開始準備中", { exact: true }).waitFor();
  await open();
  check(await panel().getByRole("button", { name: "議事録を生成", exact: true }).isDisabled(), "remount allowed duplicate admission");
  await page.evaluate(() => { window.__mon.startMode = "success"; window.__mon.startRelease(); });
  await page.waitForFunction(() => window.__mon.jobs.length === 1);
  check(await page.evaluate(() => window.__mon.plainCalls.filter(c => c.cmd === "start_markdown_minutes_generation").length === 1), "admission was duplicated");
  await page.evaluate(() => window.__mon.completePlain());
  await panel().getByRole("heading", { name: "生成された議事録", exact: true }).waitFor();
  return { passed: 5, checks: ["confirmation releases before delayed IPC", "preparation appears immediately", "navigation works while admission waits", "navigation back cannot duplicate a pending request", "one accepted job publishes normally"] };
}
