async page => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const boot = async (variant = "windows", productName = "MinutesOn") => {
    await page.goto(`http://127.0.0.1:1420/?fixture=${variant}&productName=${encodeURIComponent(productName)}`);
    await page.getByRole("main", { name: "起動診断", exact: true }).waitFor();
    await page.getByRole("button", { name: "起動をやり直す", exact: true }).waitFor();
  };
  const commands = () => page.evaluate(() => window.startupFixture.calls.map(call => call.command));
  const retry = () => page.getByRole("button", { name: "起動をやり直す", exact: true });
  const quit = () => page.getByRole("button", { name: "このアプリを終了", exact: true });
  try {
    for (const variant of ["windows", "blocked"]) {
      await boot(variant);
      check(await page.getByRole("button", { name: "MONを再起動して復旧", exact: true }).count() === 0, "removed process-termination control remains");
      check(await page.getByRole("button", { name: "もう一度開く", exact: true }).count() === 0, "removed activation handshake remains");
      check((await commands()).every(command => ["get_startup_report", "get_startup_reset_state"].includes(command)), "startup automatically invoked a mutation");
      check(await page.locator(".app-layout,.mini-shell").count() === 0, "blocked startup mounted workspace");
      await page.evaluate(() => { window.startupFixture.holdRetry = true; });
      await retry().click();
      await page.waitForFunction(() => window.startupFixture.pendingRetry);
      check(await retry().isDisabled() && await quit().isDisabled(), "pending restart allowed another exit request");
      check((await commands()).filter(command => command === "retry_startup").length === 1, "restart was duplicated");
      await page.evaluate(() => window.startupFixture.pendingRetry());
      results.push(`${variant}: diagnostic remains read-only until explicit restart; pending restart is exclusive`);
    }
    await boot("windows", "MinutesOn QA");
    await page.getByText("保存先の検証に失敗したため、起動を中止しました。", { exact: true }).waitFor();
    check(await page.title() === "MinutesOn QA — 会議記録", "QA title did not use native product name");
    await page.evaluate(() => { window.startupFixture.retryFail = true; });
    await retry().click();
    await page.getByRole("alert").filter({ hasText: "再起動の受付を確認できません" }).waitFor();
    check((await page.getByRole("alert").filter({ hasText: "再起動の受付を確認できません" }).innerText()).includes("MinutesOn QAを終了"), "QA restart guidance did not use native product name");
    check(await retry().isDisabled() && await quit().isEnabled(), "uncertain restart was resendable or blocked normal exit");
    check((await commands()).filter(command => command === "retry_startup").length === 1, "lost response replayed restart");
    await quit().click();
    check((await commands()).filter(command => command === "quit_blocked_startup").length === 1, "explicit exit was not preserved");
    results.push("uncertain restart cannot be resent; explicit exit remains available");
    return { passed: results.length, results };
  } finally { await page.goto("about:blank"); }
}
