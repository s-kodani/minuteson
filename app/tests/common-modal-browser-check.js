async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const dialog = () => page.getByRole("dialog", { name: "対象の変更確認", exact: true });
  const open = async () => { await page.getByRole("button", { name: "確認を開く", exact: true }).click(); await dialog().waitFor(); await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "確認をキャンセル"); };
  const closed = async () => { await dialog().waitFor({ state: "hidden" }); await page.waitForFunction(() => document.activeElement?.textContent === "確認を開く"); };
  try {
  check(await page.evaluate(() => [...document.querySelectorAll("dialog")].every(element => !element.hasAttribute("aria-modal"))), "closed dialogs declare independent modality");
  await open();
  check(await dialog().evaluate(element => element.matches(":modal") && !element.hasAttribute("aria-modal")), "native modal semantics missing");
  results.push("showModal exposes native modality without permanent ARIA on closed or queued dialogs");
  await page.keyboard.press("Enter"); await closed();
  check(await page.evaluate(() => window.modalFixture.commands) === 0, "initial Enter executed a command");
  results.push("safe initial cancellation and Enter, StrictMode opener restoration without commands");

  await open();
  for (let i = 0; i < 18; i++) { await page.keyboard.press("Tab"); check(await page.evaluate(() => !!document.activeElement?.closest("dialog[open]")), "Tab escaped modal"); }
  for (let i = 0; i < 18; i++) { await page.keyboard.press("Shift+Tab"); check(await page.evaluate(() => !!document.activeElement?.closest("dialog[open]")), "reverse Tab escaped modal"); }
  await page.getByRole("textbox", { name: "背景の入力" }).evaluate(e => e.focus());
  check(await page.evaluate(() => !!document.activeElement?.closest("dialog[open]")), "inert background gained focus");
  await page.keyboard.press("Escape"); await closed();
  results.push("native forward/backward focus containment, inert background, Escape cancellation");

  await open(); let box = await dialog().boundingBox();
  await page.mouse.click(box.x + 2, box.y + 2); check(await dialog().isVisible(), "padding click dismissed");
  await page.mouse.move(box.x + 2, box.y + 2); await page.mouse.down(); await page.mouse.move(1, 1); await page.mouse.up();
  check(await dialog().isVisible(), "inside-to-outside drag dismissed");
  await page.mouse.click(1, 1); await closed();
  results.push("only backdrop-origin click cancels, padding and dragging out preserve review");

  await open(); await dialog().getByRole("button", { name: "対象を変更する", exact: true }).click();
  await page.waitForFunction(() => window.modalFixture.commands === 1);
  // The fixture increments its command count before React commits the busy
  // state. Start cancellation probes only after the native modal is locked.
  await page.waitForFunction(() => {
    const review = document.querySelector('dialog[aria-labelledby="fixture-title"]');
    return review?.matches(":modal") && review.querySelector('[aria-label="確認をキャンセル"]')?.disabled === true;
  });
  check(await dialog().evaluate(element => element.contains(document.activeElement) && !document.activeElement.matches(":disabled")), "busy modal lost its keyboard guard after disabling the focused command");
  for (let i = 0; i < 5; i++) await page.keyboard.press("Escape");
  await page.mouse.click(1, 1);
  check(await dialog().isVisible() && await dialog().getByRole("button", { name: "確認をキャンセル", exact: true }).isDisabled(), "busy modal dismissed");
  check(await page.evaluate(() => window.modalFixture.commands) === 1, "busy command duplicated");
  await page.evaluate(() => window.modalFixture.finish());
  await page.waitForFunction(() => document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled === false);
  await page.keyboard.press("Escape"); await closed();
  results.push("busy and unknown-result guard blocks dismissal and repeat commands until controller confirms");

  await open(); await dialog().getByRole("button", { name: "次の確認も要求", exact: true }).click();
  check(await page.locator("dialog[open]").count() === 1, "modals stacked");
  await page.keyboard.press("Escape");
  await page.getByRole("dialog", { name: "次の対象の確認", exact: true }).waitFor();
  check(await page.locator("dialog[open]").count() === 1, "queued modal stacked after close");
  await page.keyboard.press("Escape"); await closed();
  results.push("simultaneous review requests are never stacked; next opens after actual close");

  await open(); await page.evaluate(() => window.modalFixture.closeWhileLocked()); await dialog().waitFor({ state: "hidden" });
  check(await page.getByRole("button", { name: "確認を開く", exact: true }).isDisabled(), "fixture failed to lock opener");
  await page.evaluate(() => window.modalFixture.finish()); await closed();
  results.push("opener focus waits for actual controller unlock rather than focusing a disabled target");

  for (const [width, height] of [[960, 720], [360, 640]]) {
    await page.setViewportSize({ width, height }); await open();
    box = await dialog().boundingBox();
    check(box.x >= 23.9 && box.x + box.width <= width - 23.9 && box.y >= 23.9 && box.y + box.height <= height - 23.9, "modal clipped or lacks 24px margins");
    check(await dialog().evaluate(e => e.scrollWidth <= e.clientWidth + 1), "modal content overflowed horizontally");
    const cancel = await dialog().getByRole("button", { name: "確認をキャンセル", exact: true }).boundingBox();
    check(cancel.y >= box.y && cancel.y + cancel.height <= box.y + box.height, "initial cancellation hidden by long review");
    await page.screenshot({ path: `common-modal-${width}.png` });
    await page.keyboard.press("Escape"); await closed();
  }
  check(await page.getByRole("textbox", { name: "背景の入力" }).inputValue() === "保持する内容", "background content changed");
  results.push("480px review and small window margins, body scroll, visible cancellation, untouched background content");
  await page.getByRole("button", { name: "確認を開く", exact: true }).focus();
  await page.evaluate(() => window.modalFixture.openBlocked()); await dialog().waitFor();
  check(await dialog().getByRole("heading", { name: "対象の変更確認", exact: true }).evaluate(e => e === document.activeElement), "busy opening lacks safe heading focus");
  await page.evaluate(() => window.modalFixture.finish());
  await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "確認をキャンセル");
  await page.keyboard.press("Enter"); await closed();
  results.push("initial read-busy heading moves to cancellation after readiness, without executing a command");
  await page.getByRole("button", { name: "確認を開く", exact: true }).focus();
  await page.evaluate(() => window.modalFixture.openBlocked()); await dialog().waitFor();
  await dialog().getByRole("textbox", { name: "確認欄", exact: true }).focus();
  await page.evaluate(() => window.modalFixture.finish());
  await page.waitForFunction(() => document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled === false);
  check(await dialog().getByRole("textbox", { name: "確認欄", exact: true }).evaluate(e => e === document.activeElement), "readiness stole user-selected focus");
  await page.keyboard.press("Escape"); await closed();
  results.push("initial readiness never steals focus after user moves into a field");
  await open();
  await page.getByRole("button", { name: "同じ確認内の用語設定を開く", exact: true }).click();
  await page.getByRole("textbox", { name: "切り替えで保持する用語", exact: true }).fill("点検日・入力保持");
  check(await page.locator("dialog[open]").count() === 1, "persistent settings introduced another native modal");
  await page.getByRole("button", { name: "切り替え設定内で確認を開く", exact: true }).click();
  check(await page.locator("dialog[open]").count() === 1, "persistent settings confirmation stacked native modals");
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.activeElement?.textContent === "切り替え設定内で確認を開く");
  check(await page.getByRole("textbox", { name: "切り替えで保持する用語", exact: true }).inputValue() === "点検日・入力保持", "confirmation cancellation remounted persistent fields");
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.activeElement?.textContent === "同じ確認内の用語設定を開く");
  await page.getByRole("button", { name: "同じ確認内の用語設定を開く", exact: true }).click();
  check(await page.getByRole("textbox", { name: "切り替えで保持する用語", exact: true }).inputValue() === "点検日・入力保持", "reopening remounted persistent fields");
  await page.keyboard.press("Escape"); await page.keyboard.press("Escape"); await closed();
  results.push("one native boundary keeps uncontrolled secondary fields through confirmation cancellation, close and reopen");

  await open(); await page.evaluate(() => window.modalFixture.removeOpenerAndClose());
  await dialog().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "共通モーダル検証");
  results.push("removed review opener falls back to its existing panel heading without restoring a detached node");
  return { passed: results.length, results };
  } catch (cause) {
    throw Error(`after ${results.length} completed cases (${results.at(-1) ?? "opening"}): ${cause}; active=${await page.evaluate(() => document.activeElement?.outerHTML)}; body=${await page.locator('body').innerText()}`);
  }
}
