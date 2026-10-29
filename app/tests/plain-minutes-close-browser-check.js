async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const dialog = () => page.locator("dialog[open]");
  const input = () => page.getByRole("textbox", { name:"議事録のMarkdown本文", exact:true });
  const finished = () => page.evaluate(() => window.__mon.calls.filter(c => c.cmd === "finish_main_close").length);
  const saved = () => page.evaluate(() => window.__mon.plainCalls.filter(c => c.cmd === "save_minutes_markdown").length);
  const request = () => page.evaluate(() => window.__mon.emitPlain("main-close-requested", { quit:true }));
  const boot = async () => {
    await page.goto("http://127.0.0.1:1420/");
    await page.getByRole("button", { name:"議事録UI確認", exact:true }).click();
    await page.getByRole("tab", { name:"議事録", exact:true }).click();
    await page.getByRole("button", { name:"議事録を編集", exact:true }).click();
    await input().fill("# 終了前の人間の入力\n");
  };
  const review = async () => {
    await request();
    await dialog().getByRole("heading", { name:"議事録の未保存の編集", exact:true }).waitFor();
  };
  await boot(); await review();
  await dialog().getByRole("button", { name:"確認をキャンセル", exact:true }).click();
  await dialog().waitFor({state:"hidden"});
  check(await finished() === 0 && await saved() === 0 && await input().inputValue() === "# 終了前の人間の入力\n", "quit cancellation lost or implicitly saved human input");
  check(await page.getByRole("alert").count() === 0, "normal quit cancellation displayed a failure toast");
  await page.waitForFunction(() => document.activeElement === document.querySelector('textarea[aria-label="議事録のMarkdown本文"]'));
  await page.keyboard.press("End"); await page.keyboard.type("取消後の追記");
  check((await input().inputValue()).includes("取消後の追記"), "quit cancellation did not return to an editable input");
  results.push("native quit cancellation keeps plain Markdown input without saving or closing");

  await review(); await dialog().getByRole("button", { name:"編集を破棄して続ける", exact:true }).click();
  await page.waitForFunction(() => window.__mon.calls.some(c => c.cmd === "finish_main_close"));
  check(await finished() === 1 && await saved() === 0 && await page.evaluate(() => window.__mon.plain.m1.markdown.includes("人間の保存済み追記")), "explicit discard overwrote saved body or duplicated close");
  results.push("explicit quit discard preserves saved body and closes once");

  await boot(); await page.evaluate(() => window.__mon.plainMode = "held"); await review();
  await dialog().getByRole("button", { name:"保存して続ける", exact:true }).click();
  await page.waitForFunction(() => window.__mon.plainCalls.some(c => c.cmd === "save_minutes_markdown"));
  await request(); await page.keyboard.press("Escape");
  check(await finished() === 0 && await saved() === 1 && await dialog().isVisible(), "quit passed pending save or repeated the write");
  await page.evaluate(() => window.__mon.plainRelease());
  await page.waitForFunction(() => window.__mon.calls.some(c => c.cmd === "finish_main_close"));
  check(await finished() === 1 && await page.evaluate(() => window.__mon.plain.m1.markdown) === "# 終了前の人間の入力\n", "quit did not await exact saved body");
  results.push("save-before-quit waits for actual write, blocks Escape and duplicate quit, then closes once");

  await boot(); await page.evaluate(() => window.__mon.plainMode = "failure"); await review();
  await dialog().getByRole("button", { name:"保存して続ける", exact:true }).click();
  await dialog().getByRole("alert").filter({hasText:"処理を完了できませんでした"}).first().waitFor();
  check(await finished() === 0 && await saved() === 1, "failed human save closed app or retried implicitly");
  await dialog().locator(":scope > .common-modal-dismiss-row > [data-dialog-cancel]").click();
  await page.waitForFunction(() => document.activeElement === document.querySelector('textarea[aria-label="議事録のMarkdown本文"]'));
  check(await input().inputValue() === "# 終了前の人間の入力\n", "failed quit save lost human input");
  results.push("failed save blocks native quit and retains human input after dismissing failure");
  return results;
}
