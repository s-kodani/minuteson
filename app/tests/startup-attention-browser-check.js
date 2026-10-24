async (page, capturePath = path => path) => {
  const results = [], check = (ok, msg) => { if (!ok) throw Error(msg); };
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch(); const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/startup-attention-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  const boot = async (variant = "blocked", dedicated = false) => { await page.goto(`http://127.0.0.1:1420/?fixture=${variant}${dedicated ? "&window=startup-attention" : ""}`); await page.getByRole("main", { name: "起動診断", exact: true }).waitFor(); await page.waitForFunction(() => window.startupFixture?.calls.length > 0); };
  const reload = () => page.getByRole("button", { name: "保持した診断だけを再読込", exact: true });
  const quit = () => page.getByRole("button", { name: "このアプリを終了", exact: true });
  const commands = () => page.evaluate(() => window.startupFixture.calls.map(c => c.command));
  const noWorkspace = async () => { check(await page.locator(".app-layout,.mini-shell,.toast-host").count() === 0, "workspace/provider UI mounted while denied"); check((await commands()).every(c => ["get_startup_report", "quit_blocked_startup"].includes(c)), "attention invoked a normal desktop command"); };

  await boot(); await page.getByRole("heading", { name: "保存先の検証に失敗したため、通常の起動を中止しました", exact: true }).waitFor();
  check((await page.getByLabel("保持した起動診断", { exact: true }).innerText()).includes("<img src=x onerror=alert(1)>"), "raw cause was not retained as text");
  check(await page.locator(".startup-attention img").count() === 0, "diagnostic rendered as HTML"); await noWorkspace();
  check((await page.locator("main").innerText()).includes("通常の起動検証が既に更新した状態") && (await page.locator("main").innerText()).includes("修復成功の保証ではありません"), "guidance promises unchanged storage/repair");
  const before = await page.evaluate(() => JSON.stringify(window.startupFixture.report)); await reload().click();
  check(await page.evaluate(() => JSON.stringify(window.startupFixture.report)) === before, "diagnostic read changed native report"); await noWorkspace();
  results.push("normal main root gates all providers; cause is plain text; reread does not repair or mutate");

  await page.evaluate(() => { window.startupFixture.readFail = true; }); await reload().click();
  await page.getByRole("alert").filter({ hasText: "合成診断の読込失敗" }).waitFor();
  check(await page.getByLabel("保持した起動診断", { exact: true }).count() === 1 && await quit().isEnabled(), "read failure erased original blocked report");
  await page.evaluate(() => { window.startupFixture.readFail = false; window.startupFixture.report = { schemaVersion: 1, phase: "ready", failure: null }; }); await reload().click();
  await page.getByRole("alert").filter({ hasText: "確定済みの起動診断が変更" }).waitFor(); await noWorkspace();
  results.push("read failure preserves cause; contradictory ready reply cannot unlock blocked workspace");

  for (const variant of ["read-fail", "invalid"]) {
    await boot(variant); await page.getByRole("alert").waitFor();
    await noWorkspace(); check(await quit().count() === 0, "unverified admission enabled special failure exit");
    check((await commands()).length === 1, "unknown startup automatically retried");
    if (variant === "read-fail") { await page.evaluate(() => { window.startupFixture.readFail = false; }); await reload().click(); await quit().waitFor(); }
  }
  results.push("unreadable and forged reports fail closed; manual read may verify original failure");

  for (const phase of ["ready", "blocked"]) {
    await page.goto(`http://127.0.0.1:1420/?fixture=transition-${phase}`);
    await page.getByRole("main", { name: "起動確認", exact: true }).waitFor();
    const center = await page.getByRole("status").evaluate(element => {
      const bounds = element.getBoundingClientRect();
      return { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2, width: innerWidth, height: innerHeight };
    });
    check(Math.abs(center.x - center.width / 2) < 2 && Math.abs(center.y - center.height / 2) < 2,
      "startup message is not centered in the viewport");
    check(await page.getByRole("heading", { name: "保存先の起動確認", exact: true }).count() === 0,
      "normal pending startup showed the diagnostic page");
    check(await page.getByRole("main", { name: "通常画面", exact: true }).count() === 0,
      "workspace mounted before confirmed admission");
    if (phase === "ready") await page.getByRole("main", { name: "通常画面", exact: true }).waitFor();
    else { await quit().waitFor(); await noWorkspace(); }
    const settled = (await commands()).length;
    await page.waitForTimeout(450);
    check((await commands()).length === settled && settled >= 2,
      "startup did not reread pending memory or kept polling a settled result");
    check((await commands()).every(c => c === "get_startup_report"), "pending startup invoked storage work");
  }
  results.push("pending startup reads memory until ready/blocked, then stops; normal startup needs no confirmation");

  await boot("selection"); await page.getByText("選択先は未確定です。エラー本文から推測しません。", { exact: true }).waitFor(); await noWorkspace();
  check((await page.locator("main").innerText()).includes("保存先の起動設定の検証"), "wrong failure stage displayed");
  results.push("selection refusal has correct stage and does not invent a selected root");

  await boot("blocked", true); await quit().waitFor(); await page.evaluate(() => { window.startupFixture.quitFail = true; window.startupFixture.delay = true; });
  await quit().click(); check(await quit().isDisabled() && await reload().isDisabled(), "quit response wait allowed repeated actions");
  await page.evaluate(() => { window.startupFixture.pendingQuit(); });
  await page.getByRole("alert").filter({ hasText: "終了の受付を確認できません" }).waitFor();
  check((await commands()).filter(c => c === "quit_blocked_startup").length === 1, "lost quit automatically replayed"); await noWorkspace();
  await page.evaluate(() => { window.startupFixture.delay = false; window.startupFixture.quitFail = false; }); await quit().click();
  check((await commands()).filter(c => c === "quit_blocked_startup").length === 2, "explicit quit retry missing");
  results.push("failure-only quit has single admission and explicit retry after response loss");

  await boot("long"); await page.setViewportSize({ width: 360, height: 740 }); await page.emulateMedia({ colorScheme: "dark" });
  await page.getByText("長い診断の先頭だけを表示しています。完全な診断ではありません。", { exact: true }).waitFor();
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), "long diagnosis overflows small width");
  check(await page.getByLabel("保持した起動診断", { exact: true }).evaluate(e => e.scrollHeight > e.clientHeight), "long cause cannot scroll");
  await quit().scrollIntoViewIfNeeded(); check(await quit().isVisible(), "long diagnosis hides reachable exit");
  await page.screenshot({ path: capturePath("startup-attention-360-dark.png"), fullPage: true }); await noWorkspace(); await page.emulateMedia({ colorScheme: "light" });
  results.push("long truncated diagnosis scrolls, wraps and leaves exit reachable at 360px in dark mode");

  await boot("ready", true); await page.getByRole("link", { name: "通常画面を開く", exact: true }).waitFor();
  check(await quit().count() === 0 && (await commands()).length === 1, "ready attention used failure-only exit or workspace command");
  results.push("ready read-only attention never uses failure exit; normal navigation requires explicit input");
  return { passed: results.length, results };
}
