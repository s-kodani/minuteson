async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const trigger = (name = "PCの音声") => page.getByRole("button", { name: `${name}の説明を開く`, exact: true });
  const popup = (name = "PCの音声") => page.getByRole("dialog", { name, exact: true });
  const close = async (name = "PCの音声") => { await page.keyboard.press("Escape"); await popup(name).waitFor({ state: "hidden" }); check(await trigger(name).evaluate(e => e === document.activeElement), "focus not returned to info icon"); };
  const open = async (name = "PCの音声") => { await trigger(name).click(); await popup(name).waitFor(); };
  await trigger().focus(); await page.keyboard.press("Enter"); await popup().waitFor();
  check(await trigger().getAttribute("aria-expanded") === "true", "expanded state missing");
  check(await popup().getAttribute("aria-modal") === "false", "help made background modal");
  const icon = await trigger().boundingBox(); check(icon.width >= 32 && icon.height >= 32, "small touch target");
  await close(); await page.keyboard.press("Space"); await popup().waitFor(); await close();
  results.push("click/Enter/Space access, non-modal description and 32px target, Escape returns focus");

  await open(); await page.mouse.click(700, 500); await popup().waitFor({ state: "hidden" });
  check(await trigger().evaluate(e => e === document.activeElement), "outside click focus missing");
  await open(); await popup().getByRole("button", { name: "PCの音声の説明を閉じる" }).click();
  check(await trigger().evaluate(e => e === document.activeElement), "close focus missing");
  results.push("outside click and explicit close restore trigger without changing background input");

  await open(); await page.keyboard.press("Tab");
  check(await popup().getByRole("link", { name: "詳しい説明", exact: true }).evaluate(e => e === document.activeElement), "detail link not reachable by Tab");
  await page.keyboard.press("Enter"); await popup().getByRole("link", { name: "説明内のリンク" }).waitFor();
  await page.keyboard.press("Tab"); await page.keyboard.press("Enter");
  check(await page.evaluate(() => window.infoFixture.detailActions) === 1, "nested detail link not keyboard operable");
  check(await popup().evaluate(e => e.scrollHeight > e.clientHeight), "long description did not scroll");
  await close(); await open(); check(await popup().getByRole("link", { name: "詳しい説明", exact: true }).getAttribute("aria-expanded") === "false", "details did not reset on reopening");
  await close(); results.push("short summary, keyboard-operated detail link, scrollable long explanation and fresh reopen");

  await open(); await open("話者の人数"); await popup().waitFor({ state: "hidden" });
  check(await page.locator(".info-popover:popover-open").count() === 1, "help popovers stacked");
  check(await popup("話者の人数").getByRole("button").evaluate(e => e === document.activeElement), "switch returned to old trigger");
  await close("話者の人数"); results.push("one info at a time without focus returning to prior help");

  for (const [width, height, x, y] of [[960, 720, 10, 60], [960, 720, 760, 660], [360, 640, 160, 570], [280, 300, 40, 210]]) {
    await page.setViewportSize({ width, height }); await page.evaluate(([a, b]) => window.infoFixture.setAnchor(a, b), [x, y]);
    await open();
    const bounds = await popup().boundingBox(), viewWidth = await page.evaluate(() => document.documentElement.clientWidth);
    check(bounds.x >= 7.9 && bounds.x + bounds.width <= viewWidth - 7.9 && bounds.y >= 7.9 && bounds.y + bounds.height <= height - 7.9, `clipped ${width}x${height} popup`);
    check(await popup().evaluate(e => e.scrollWidth <= e.clientWidth + 1), "help horizontal overflow");
    if (y > height / 2) check(await popup().getAttribute("data-side") === "above", "bottom-edge popup failed to flip");
    await page.screenshot({ path: `info-popover-${width}.png` }); await close();
  }
  results.push("corner flipping and viewport bounds at 960, 360 and 280px with classic scrollbar");

  await page.setViewportSize({ width: 960, height: 720 }); await page.evaluate(() => window.infoFixture.setAnchor(48, 80)); await open();
  const before = await popup().boundingBox(); await page.evaluate(() => window.infoFixture.setAnchor(160, 120));
  await page.waitForFunction(({ x, y }) => {
    const r = document.querySelector(".info-popover:popover-open")?.getBoundingClientRect();
    return r && r.x > x && r.y > y;
  }, before);
  const after = await popup().boundingBox(); check(after.x > before.x && after.y > before.y, "anchor update left popup behind");
  await page.setViewportSize({ width: 400, height: 350 });
  await page.waitForFunction(() => { const e = document.querySelector(".info-popover:popover-open"), r = e?.getBoundingClientRect(); return r && r.right <= document.documentElement.clientWidth - 7.9 && r.bottom <= document.documentElement.clientHeight - 7.9; });
  await page.setViewportSize({ width: 960, height: 720 });
  await page.evaluate(() => window.infoFixture.update());
  check(await popup().getByRole("button").evaluate(e => e === document.activeElement), "state update stole focus");
  await page.getByRole("textbox", { name: "背景の入力" }).focus();
  await page.evaluate(() => window.infoFixture.update());
  check(await page.getByRole("textbox", { name: "背景の入力" }).evaluate(e => e === document.activeElement), "help update stole background focus");
  await close(); results.push("anchor and viewport updates position help without background progress stealing focus");

  await open(); await page.evaluate(() => window.infoFixture.confirm());
  const modal = page.getByRole("dialog", { name: "対象の変更確認", exact: true }); await modal.waitFor(); await popup().waitFor({ state: "hidden" });
  check(await modal.getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "help stole confirmation focus");
  await open("確認内の補足"); await close("確認内の補足"); check(await modal.isVisible(), "Escape closed parent confirmation");
  check((await modal.innerText()).includes("削除したデータは復元できません"), "irreversible warning hidden in help");
  // The input below the trigger is legitimately covered by its help popup.
  // Click the exposed modal heading before interacting with that input.
  await open("確認内の補足"); await modal.getByRole("heading", { name:"対象の変更確認",exact:true }).click(); await popup("確認内の補足").waitFor({ state: "hidden" });
  await modal.getByRole("textbox", { name: "確認の入力" }).click();
  check(await modal.isVisible(), "outside help click closed parent modal");
  await page.keyboard.press("Escape"); await modal.waitFor({ state: "hidden" });
  results.push("confirmation takes precedence; nested help Escape/outside close only help and retain visible loss warning");

  await open(); await page.evaluate(() => window.infoFixture.setHidden(true)); await popup().waitFor({ state: "hidden" });
  await page.evaluate(() => window.infoFixture.setHidden(false)); await open();
  await page.evaluate(() => window.infoFixture.remove()); await popup().waitFor({ state: "detached" });
  check(await page.locator(".info-popover:popover-open").count() === 0, "removed component left a popover");
  check(await page.getByRole("textbox", { name: "背景の入力" }).inputValue() === "保持する内容", "help changed input");
  results.push("hidden/removed trigger tears down overlay and listeners; background input unchanged");
  return { passed: results.length, results };
}
