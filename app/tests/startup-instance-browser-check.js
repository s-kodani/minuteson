async page => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const boot = async (variant = "windows") => {
    await page.goto(`http://127.0.0.1:1420/?fixture=${variant}`);
    await page.getByRole("heading", { name: "MONの起動と復旧", exact: true }).waitFor();
    await page.getByRole("button", { name: "起動をやり直す", exact: true }).waitFor();
  };
  const count = command => page.evaluate(command => window.startupFixture.calls.filter(call => call.command === command).length, command);
  const restart = () => page.getByRole("button", { name: "MONを再起動して復旧", exact: true });
  const modal = () => page.getByRole("dialog", { name: "既存のMONを終了して復旧しますか？", exact: true });
  const accept = () => modal().getByRole("button", { name: "確認して再起動する", exact: true });
  try {
    await boot(); await restart().waitFor();
    await page.evaluate(() => window.startupFixture.activate("verified-activation-request"));
    await page.waitForFunction(() => window.startupFixture.calls.some(call => call.command === "acknowledge_startup_activation"));
    check(await page.evaluate(() => window.startupFixture.calls.find(call => call.command === "acknowledge_startup_activation").args.requestId === "verified-activation-request"), "activation acknowledgement lost its request identity");
    results.push("mounted Windows UI acknowledges the exact native activation request");
    check(await count("recover_startup_instance") === 0 && await count("retry_startup") === 0, "startup executed recovery without consent");
    check(await page.locator(".sidebar").count() === 0, "recovery mounted storage workspace");
    await restart().click(); await modal().waitFor(); await page.keyboard.press("Enter"); await modal().waitFor({ state: "hidden" });
    check(await count("recover_startup_instance") === 0, "initial Enter terminated owner");
    results.push("blocked startup exposes actionable recovery; initial Enter cancels without termination");
    await page.getByRole("button", { name: "もう一度開く", exact: true }).click();
    await page.getByRole("alert").filter({ hasText: "既存のMONを開けませんでした" }).waitFor();
    check(await count("recover_startup_instance") === 0, "activation failure automatically terminated owner");
    await page.evaluate(() => { window.startupFixture.opened = true; });
    await page.getByRole("button", { name: "もう一度開く", exact: true }).click();
    await page.waitForFunction(() => window.startupFixture.calls.filter(c => c.command === "open_existing_instance").length === 2);
    results.push("failed activation can retry safely without issuing restart");

    await boot(); await restart().click(); await accept().waitFor();
    await page.evaluate(() => { window.startupFixture.holdRecover = true; });
    await accept().dblclick(); await modal().getByRole("status").waitFor(); await page.keyboard.press("Escape");
    check(await modal().isVisible() && await count("recover_startup_instance") === 1 && await accept().isDisabled(), "in-flight recovery dismissed or repeated");
    check(await page.evaluate(() => window.startupFixture.calls.find(c => c.command === "recover_startup_instance").args.targetId === window.startupFixture.instance.targetId), "confirmation did not bind exact target");
    await page.evaluate(() => window.startupFixture.pendingRecover()); await modal().waitFor({ state: "hidden" });
    results.push("explicit confirmation sends one target-bound restart and blocks Escape while pending");

    await boot(); await restart().click(); await accept().waitFor();
    await page.evaluate(() => { window.startupFixture.failRecover = true; }); await accept().click();
    await modal().getByRole("alert").waitFor(); check(await accept().isDisabled() && await count("recover_startup_instance") === 1, "uncertain recovery was resendable");
    await page.keyboard.press("Escape"); await modal().waitFor({ state: "hidden" });
    check(await count("recover_startup_instance") === 1 && await page.locator(".sidebar").count() === 0, "lost acknowledgement unlocked workspace or resent operation");
    results.push("lost recovery acknowledgement retains recovery UI without automatic replay");

    await boot("windows-no-owner"); check(await restart().count() === 0, "unknown lock owner could be terminated");
    await page.getByRole("button", { name: "起動をやり直す", exact: true }).click();
    await page.waitForFunction(() => window.startupFixture.calls.some(c => c.command === "retry_startup"));
    check(await count("recover_startup_instance") === 0, "storage retry terminated an unknown owner");
    results.push("unknown owner allows fresh admission retry without unsafe termination");

    await boot("windows-invalid"); await page.getByRole("alert").filter({ hasText: "復旧対象を確認できません" }).waitFor();
    check(await restart().count() === 0 && await count("recover_startup_instance") === 0, "invalid native target enabled stop");
    results.push("unverified target cannot enable process termination");

    await boot(); await restart().click(); await modal().waitFor(); await page.setViewportSize({ width: 360, height: 640 });
    const bounds = await modal().boundingBox(); check(bounds.x >= 0 && bounds.x + bounds.width <= 360 && await modal().evaluate(e => e.scrollWidth <= e.clientWidth + 1), "recovery dialog has horizontal overflow");
    await page.screenshot({ path: "output/windows-qa-work/startup-recovery-confirmation-360.png" }); await page.keyboard.press("Escape");
    results.push("restart confirmation remains usable at 360px without horizontal overflow");
    return { passed: results.length, results };
  } finally { await page.goto("about:blank"); await page.setViewportSize({ width: 1280, height: 720 }); }
}
