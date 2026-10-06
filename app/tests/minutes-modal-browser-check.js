// CLI-first integration check after minutes-browser-fixture.js bootstrap.
async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const panel = () => page.getByRole("region", { name: "議事録生成と候補確認", exact: true });
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const restore = () => panel().getByRole("button", { name: "議事録直前版を確認・復元", exact: true });
  const count = command => page.evaluate(c => window.__mon.calls.filter(x => x.cmd === c).length, command);
  const boot = async () => { await page.reload(); await page.getByRole("button", { name: "開く", exact: true }).first().click(); await restore().waitFor(); };
  const close = async (name, trigger) => { await modal(name).waitFor({ state: "hidden" }); await page.waitForFunction(() => !document.querySelector("dialog[open]")); check(await trigger.evaluate(e => e === document.activeElement), "minutes opener focus missing"); };
  await boot(); const originalDraft = await page.evaluate(() => JSON.stringify(window.__mon.drafts.a));
  await restore().click(); const previous = "議事録直前版の復元確認 #1";
  await modal(previous).waitFor();
  check(await modal(previous).getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "minutes safe focus missing");
  check((await modal(previous).innerText()).includes("保存済みの手動追記を保持します。"), "comparison omitted saved draft");
  await page.keyboard.press("Escape"); await close(previous, restore());
  check(await count("restore_minutes_version") === 0 && await page.evaluate(() => JSON.stringify(window.__mon.drafts.a)) === originalDraft, "Escape changed minutes head or saved edit");
  results.push("previous minutes comparison includes saved draft; Escape is read-only with opener focus");

  await restore().click(); const box = await modal(previous).boundingBox();
  await page.mouse.click(box.x - 10, box.y + 10); await close(previous, restore());
  check(await count("restore_minutes_version") === 0, "backdrop restored minutes");
  results.push("minutes backdrop cancellation never publishes a version");

  await page.setViewportSize({ width: 360, height: 640 }); await restore().click();
  const small = await modal(previous).boundingBox();
  check(small.x >= 23.9 && small.x + small.width <= 336.1 && await modal(previous).evaluate(e => e.scrollWidth <= e.clientWidth + 1), "minutes comparison overflows small dialog");
  await page.screenshot({ path: "minutes-modal-360.png" });
  await page.keyboard.press("Escape"); await close(previous, restore());
  results.push("production minutes documents and evidence controls fit 360px modal margins");

  await page.setViewportSize({ width: 1200, height: 900 }); await restore().click();
  await page.evaluate(() => { window.__mon.confirmations = []; window.confirm = message => { window.__mon.confirmations.push(message); return true; }; });
  await modal(previous).getByRole("button", { name: "この議事録直前版へ戻す", exact: true }).click();
  await close(previous, restore());
  check(await count("restore_minutes_version") === 1 && await page.evaluate(() => window.__mon.current === "p" && window.__mon.previous === "a" && window.__mon.confirmations.length === 1), "restore changed head identity or confirmation count");
  check(await page.evaluate(() => JSON.stringify(window.__mon.drafts.a)) === originalDraft, "restore discarded old draft");
  results.push("explicit previous-version restore is single and retains saved current draft and focus");

  await boot(); await panel().getByRole("button", { name: "議事録を再生成", exact: true }).click();
  await page.waitForFunction(() => window.__mon.jobs.length === 1);
  await page.evaluate(() => window.__mon.complete());
  const review = panel().getByRole("button", { name: "議事録候補を確認", exact: true }); await review.waitFor(); await review.click();
  const candidate = "議事録候補の採用確認 #3";
  await modal(candidate).waitFor(); await page.keyboard.press("Escape"); await close(candidate, review);
  check(await count("adopt_minutes_candidate") === 0, "candidate Escape adopted minutes");
  await review.click(); await page.evaluate(() => { window.__mon.confirmations = []; window.confirm = message => { window.__mon.confirmations.push(message); return true; }; });
  await modal(candidate).getByRole("button", { name: "この議事録候補を採用", exact: true }).click();
  await modal(candidate).waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "議事録");
  check(await count("adopt_minutes_candidate") === 1 && await page.evaluate(() => window.__mon.current === "b" && window.__mon.previous === "a" && !!window.__mon.drafts.a && window.__mon.confirmations.length === 1), "adoption lost preserved edit or generated duplicate");
  results.push("candidate cancellation is read-only; single explicit adoption retains prior draft and restores panel focus");
  return { passed: results.length, results };
}
