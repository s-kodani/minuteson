async (page, capturePath = path => path) => {
  const results = [], check = (ok, msg) => { if (!ok) throw Error(msg); };
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch(); const body = (await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/startup-attention-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  const boot = async (variant = "blocked", dedicated = false) => { await page.goto(`http://127.0.0.1:1420/?fixture=${variant}${dedicated ? "&window=startup-attention" : ""}`); await page.getByRole("main", { name: "起動診断", exact: true }).waitFor(); await page.waitForFunction(() => window.startupFixture?.calls.length > 0); };
  const quit = () => page.getByRole("button", { name: "このアプリを終了", exact: true });
  const commands = () => page.evaluate(() => window.startupFixture.calls.map(c => c.command));
  const noWorkspace = async () => { check(await page.locator(".app-layout,.mini-shell").count() === 0, "workspace/provider UI mounted while denied"); check((await commands()).every(c => ["get_startup_report", "get_startup_reset_state", "quit_blocked_startup"].includes(c)), "attention invoked a normal desktop command"); };

  await boot(); await page.getByText("保存先の検証に失敗したため、起動を中止しました。", { exact: true }).waitFor();
  check((await page.getByLabel("保持した起動診断", { exact: true }).innerText()).includes("<img src=x onerror=alert(1)>"), "raw cause was not retained as text");
  check(await page.locator(".startup-attention img").count() === 0, "diagnostic rendered as HTML"); await noWorkspace();
  check(await page.getByRole("button", { name: "保持した診断だけを再読込", exact: true }).count() === 0, "refresh control remains");
  check(await page.locator(".startup-attention h1,.startup-attention dl,.startup-attention .status-card").count() === 0, "verbose diagnostic panels remain");
  check(await page.locator(".startup-attention-cause").evaluate(element => {
    const heading = element.querySelector("h2").getBoundingClientRect(), message = element.querySelector("pre").getBoundingClientRect();
    return message.top >= heading.bottom && Math.abs(heading.left - message.left) < 1;
  }), "cause label and message are not vertically aligned");
  const actions = page.locator(".startup-attention-actions > button");
  check((await actions.allTextContents()).join("|") === "起動をやり直す|データ復元|データを初期化|このアプリを終了", "wrong startup action order");
  for (const width of [1200, 640, 360, 320]) {
    await page.setViewportSize({ width, height: 800 });
    const bounds = await actions.evaluateAll(buttons => buttons.map(button => { const rect = button.getBoundingClientRect(); return { top: rect.top, right: rect.right, left: rect.left }; }));
    check(bounds.every(rect => Math.abs(rect.top - bounds[0].top) < 1 && rect.left >= 0 && rect.right <= width + 1), `actions are not one reachable row at ${width}px`);
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `startup overflows at ${width}px`);
  }
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.screenshot({ path: capturePath("startup-attention-simple.png"), fullPage: true });
  results.push("one lead, vertical cause, four actions on one row; diagnostics remain plain text without refresh");

  await page.evaluate(()=>window.startupFixture.emitResidency("residency-monitor-status","合成監視障害"));
  await page.waitForTimeout(300);
  check(await page.getByRole("alert").filter({hasText:"常駐監視: 合成監視障害"}).count()===0,"internal monitor notified during startup");
  await page.evaluate(()=>window.startupFixture.emitResidency("residency-monitor-status",null));
  await page.getByRole("alert").filter({hasText:"常駐監視: 合成監視障害"}).waitFor({state:"hidden"});
  check(await page.getByLabel("保持した起動診断", { exact: true }).count() === 1 && await quit().isEnabled(), "monitor event erased original blocked report");
  await noWorkspace();
  results.push("internal monitor events do not erase or duplicate the startup cause");

  for (const variant of ["read-fail", "invalid"]) {
    await boot(variant); await page.getByRole("alert").waitFor();
    await noWorkspace(); check(await quit().count() === 0, "unverified admission enabled special failure exit");
    check((await commands()).length === 1, "unknown startup automatically retried");
  }
  results.push("unreadable and forged reports fail closed without automatic retries");

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
    check((await commands()).every(c => ["get_startup_report", "get_startup_reset_state"].includes(c)), "pending startup invoked storage work");
  }
  results.push("pending startup reads memory until ready/blocked, then stops; normal startup needs no confirmation");

  await boot("selection"); await page.getByText("保存先選択の検証は要確認です", { exact: true }).waitFor(); await noWorkspace();
  results.push("selection refusal has correct stage and does not invent a selected root");

  await boot("blocked", true); await quit().waitFor(); await page.evaluate(() => { window.startupFixture.quitFail = true; window.startupFixture.delay = true; });
  await quit().click(); check(await actions.evaluateAll(buttons => buttons.every(button => button.disabled)), "quit response wait allowed concurrent recovery");
  await page.evaluate(() => { window.startupFixture.pendingQuit(); });
  await page.getByRole("alert").filter({ hasText: "終了の受付を確認できません" }).waitFor();
  check((await commands()).filter(c => c === "quit_blocked_startup").length === 1, "lost quit automatically replayed"); await noWorkspace();
  await page.evaluate(() => { window.startupFixture.delay = false; window.startupFixture.quitFail = false; }); await quit().click();
  check((await commands()).filter(c => c === "quit_blocked_startup").length === 2, "explicit quit retry missing");
  results.push("failure-only quit has single admission and explicit retry after response loss");

  await boot("windows");
  await page.setViewportSize({ width: 900, height: 420 });
  await page.screenshot({ path: capturePath("startup-recovery-updated.png"), fullPage: true });
  await page.getByRole("button", { name: "データ復元", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "データ復元", exact: true });
  await dialog.waitFor();
  await page.setViewportSize({ width: 900, height: 620 });
  await page.screenshot({ path: capturePath("startup-data-restore-confirmation.png"), fullPage: true });
  check((await dialog.innerText()).includes("ボイス登録、モデル、設定"), "recovery scope missing");
  check((await dialog.innerText()).includes("すべてのデータを回収・検証できない場合は初期化せず"), "all-or-nothing recovery contract missing");
  check(await dialog.getByRole("radio").count() === 0, "recovery exposed raw reset/archive choices");
  check(await page.evaluate(() => window.startupFixture.calls.find(call => call.command === "get_startup_reset_preview")?.args.restore === true), "recovery mode not bound to native preparation");
  check(!(await commands()).includes("start_startup_reset"), "recovery started before confirmation");
  await dialog.getByRole("button", { name: "キャンセル", exact: true }).click();
  check(await page.getByRole("button", { name: "データ復元", exact: true }).evaluate(button => document.activeElement === button), "recovery did not restore trigger focus");
  results.push("recovery confirms its scope and stops before reset if any collection fails");
  await page.getByRole("button", { name: "データ復元", exact: true }).click();
  await page.evaluate(() => { window.startupFixture.startReplyFail = true; });
  await dialog.getByRole("button", { name: "復元して再起動", exact: true }).click();
  const progress = dialog.getByRole("progressbar", { name: "データ復元の進捗", exact: true });
  await progress.waitFor();
  check(await progress.getAttribute("value") === null, "unknown recovery total displayed a fabricated percentage");
  check(await dialog.getByRole("button", { name: "キャンセル", exact: true }).isDisabled(), "in-flight reconstruction could be dismissed as cancelled");
  await page.evaluate(() => { window.startupFixture.resetState.message = "モデル本体と設定を回収しています"; });
  await dialog.getByRole("status").filter({ hasText: "モデル本体と設定を回収しています" }).waitFor();
  check(await dialog.getByRole("alert").filter({ hasText: "受付状態を再確認" }).count() === 0, "accepted native recovery kept the stale response-loss error");
  await page.screenshot({ path: capturePath("startup-data-restoring.png"), fullPage: true });
  check((await commands()).filter(command => command === "start_startup_reset").length === 1, "progress polling replayed recovery");
  await page.keyboard.press("Escape");
  check(await dialog.isVisible(), "Escape hid active recovery progress");
  await page.evaluate(() => { window.startupFixture.resetState.busy = false; window.startupFixture.resetState.message = "データ復元を完了できませんでした"; window.startupFixture.resetState.error = "回収できない音声があるため、元データを保持しました"; });
  await progress.waitFor({ state: "hidden" });
  await dialog.getByRole("alert").filter({ hasText: "元データを保持" }).waitFor();
  await boot("restore-running");
  await dialog.getByRole("progressbar", { name: "データ復元の進捗", exact: true }).waitFor();
  check(!(await commands()).includes("start_startup_reset"), "reconnected progress restarted recovery");
  results.push("recovery progress remains in its modal, updates the native stage and never replays the operation");

  await boot("long"); await page.setViewportSize({ width: 360, height: 740 }); await page.emulateMedia({ colorScheme: "dark" });
  await page.getByRole("heading", { name: "原因（一部のみ表示）", exact: true }).waitFor();
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
