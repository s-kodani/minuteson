async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const prompt = () => page.getByRole("dialog", { name: "対象の変更を確認", exact: true });
  const parent = () => page.getByRole("dialog", { name: "候補の比較", exact: true });
  const source = () => page.getByRole("button", { name: "変更の確認を開く", exact: true });
  const calls = () => page.evaluate(() => window.actionFixture.commands.length);
  const reset = async () => { await page.reload(); await source().waitFor(); };
  const choose = async () => { await source().click(); await prompt().waitFor(); };
  const single = async () => check(await page.locator("dialog[open]").count() === 1, "confirmations stacked");
  await choose(); await single(); check(await calls() === 0, "opening changed target");
  check((await prompt().innerText()).includes("復元できなくなります"), "loss omitted");
  check(await prompt().getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "initial focus unsafe");
  await page.keyboard.press("Enter"); await prompt().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "変更の確認を開く");
  check(await calls() === 0, "initial Enter executed");
  results.push("safe initial Enter cancellation, target and loss visible, no command before acceptance");
  await choose(); await page.keyboard.press("Escape"); await prompt().waitFor({ state: "hidden" });
  await choose(); await page.mouse.click(1, 1); await prompt().waitFor({ state: "hidden" });
  check(await calls() === 0, "Escape or backdrop executed"); results.push("Escape and outside cancellation without changes");

  await reset(); await page.evaluate(() => window.actionFixture.mode = "delay"); await choose();
  await prompt().getByRole("button", { name: "対象を変更する", exact: true }).dblclick();
  await page.waitForFunction(() => window.actionFixture.commands.length === 1);
  for (let i = 0; i < 5; i++) await page.keyboard.press("Escape");
  await page.mouse.click(1, 1); await single();
  check(await prompt().getByRole("button", { name: "確認をキャンセル", exact: true }).isDisabled(), "running request dismissed");
  check(await calls() === 1 && (await prompt().innerText()).includes("完了応答を待っています"), "running status or duplicate protection missing");
  await page.evaluate(() => window.actionFixture.complete()); await prompt().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "変更の確認を開く");
  results.push("explicit single execution, repeated Escape non-dismissal until actual completion, focus after caller unlock");

  await reset(); await choose(); await page.evaluate(() => window.actionFixture.version++);
  await prompt().getByRole("button", { name: "対象を変更する", exact: true }).click(); await prompt().getByRole("alert").waitFor();
  check(await calls() === 0 && await prompt().getByRole("button", { name: "対象を変更する", exact: true }).isDisabled(), "stale target accepted or retry allowed");
  await page.keyboard.press("Escape"); await prompt().waitFor({ state: "hidden" });
  results.push("changed target fails before action, retained error and explicit new-review requirement");
  await reset(); await choose(); await page.evaluate(() => window.actionFixture.blocked(true));
  await prompt().getByRole("button", { name: "対象を変更する", exact: true }).click(); await prompt().getByRole("alert").waitFor();
  check(await calls() === 0, "late write protection bypassed"); await page.keyboard.press("Escape");
  results.push("live write protection is rechecked at acceptance");

  await reset(); await page.evaluate(() => window.actionFixture.mode = "lost"); await choose();
  await prompt().getByRole("button", { name: "対象を変更する", exact: true }).click(); await prompt().getByRole("alert").waitFor();
  check(await calls() === 1 && await prompt().getByRole("button", { name: "対象を変更する", exact: true }).isDisabled(), "lost response replayed");
  await prompt().getByRole("button", { name: "確認を閉じる", exact: true }).click(); await prompt().waitFor({ state: "hidden" });
  check(await calls() === 1, "closing uncertainty changed command count"); results.push("lost response remains visible; no automatic repeat or cancellation of committed work");

  await reset(); await page.getByRole("button", { name: "比較を開く", exact: true }).click(); await parent().waitFor();
  await parent().getByRole("textbox", { name: "比較中の入力", exact: true }).fill("保持する手動の点検日");
  const mounts = await page.evaluate(() => window.actionFixture.mounts);
  const adopt = parent().getByRole("button", { name: "候補を採用", exact: true }); await adopt.click(); await prompt().waitFor(); await single();
  check(await page.getByRole("dialog").count() === 1, "original comparison still exposed");
  await page.keyboard.press("Escape"); await parent().waitFor();
  await page.waitForFunction(() => document.activeElement?.textContent === "候補を採用");
  check(await parent().getByRole("textbox", { name: "比較中の入力", exact: true }).inputValue() === "保持する手動の点検日" && await page.evaluate(() => window.actionFixture.mounts) === mounts, "comparison was unmounted or input lost");
  const box = await adopt.boundingBox(), bounds = await parent().boundingBox();
  check(box.y >= bounds.y && box.y + box.height <= bounds.y + bounds.height + 1, "comparison scroll not restored");
  check(await calls() === 0, "nested cancellation executed"); results.push("nested choice replaces rather than stacks; mounted input, scroll and source focus restored");

  await page.evaluate(() => window.actionFixture.mode = "delay"); await adopt.click(); await prompt().waitFor();
  await prompt().getByRole("button", { name: "対象を変更する", exact: true }).click();
  await page.evaluate(() => window.actionFixture.closeParent()); await prompt().waitFor(); await single();
  check(await calls() === 1 && await prompt().getByRole("button", { name: "確認をキャンセル", exact: true }).isDisabled(), "handoff changed execution");
  await page.evaluate(() => window.actionFixture.complete()); await prompt().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "比較を開く");
  results.push("closing the comparison transfers the same in-flight request and actual outer opener, without another command");

  await reset(); await page.setViewportSize({ width: 360, height: 640 }); await choose();
  const small = await prompt().boundingBox(); check(small.x >= 23.9 && small.x + small.width <= 336.1, "small confirmation lacks safe margins");
  for (let i = 0; i < 8; i++) { await page.keyboard.press("Tab"); check(await page.evaluate(() => !!document.activeElement?.closest("dialog[open]")), "confirmation focus escaped"); }
  await page.screenshot({ path: "action-confirmation-360.png" }); await page.keyboard.press("Escape");
  results.push("small-window margins, loss text and keyboard containment");
  await reset(); await choose(); await page.evaluate(() => window.actionFixture.removeOwner()); await prompt().waitFor({ state: "hidden" });
  check(await calls() === 0, "removed owner executed"); results.push("removed decision owner settles cancellation without acting on a new target");
  await reset(); await page.evaluate(() => { window.actionFixture.mode = "delay"; window.actionFixture.progress = true; }); await choose();
  check(await prompt().getByRole("button", { name: "準備の停止を要求する", exact: true }).count() === 0, "progress cancellation offered before acceptance");
  await prompt().getByRole("button", { name: "対象を変更する", exact: true }).click();
  await prompt().getByRole("button", { name: "準備の停止を要求する", exact: true }).dblclick();
  check(await page.evaluate(() => window.actionFixture.cancellationCalls) === 1 && await calls() === 1, "progress request repeated or restarted work");
  await page.keyboard.press("Escape"); check(await prompt().isVisible(), "progress cancellation dismissed actual work");
  await page.evaluate(() => window.actionFixture.completeCancel());
  await prompt().getByText("停止を要求しました。処理の終了応答を待っています。", { exact: true }).waitFor();
  check(await prompt().getByRole("button", { name: "準備の停止を要求する", exact: true }).isDisabled(), "acknowledged progress request remained repeatable");
  await page.evaluate(() => window.actionFixture.complete()); await prompt().waitFor({ state: "hidden" });
  results.push("explicit progress cancellation is single and keeps the original work awaited through its real completion");
  return { passed: results.length, results };
}
