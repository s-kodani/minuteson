// CLI-first integration check routed to import-browser-fixture.js.
async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const trigger = () => page.getByRole("button", { name: "音声を取り込む", exact: true });
  const modal = () => page.getByRole("dialog", { name: "音声を取り込む", exact: true });
  const count = command => page.evaluate(c => window.monImportFixture.calls.filter(x => x.command === c).length, command);
  const boot = async () => { await page.reload(); await trigger().waitFor(); };
  const open = async () => { await trigger().click(); await modal().waitFor(); await page.waitForFunction(() => window.monImportFixture.import.phase === "ready"); await modal().getByRole("button", { name: "確認をキャンセル", exact: true }).waitFor(); await page.waitForFunction(() => !document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled); };
  const closed = async () => { await modal().waitFor({ state: "hidden" }); await page.waitForFunction(() => document.activeElement?.textContent === "音声を取り込む"); };
  await boot(); await open();
  await page.evaluate(() => { window.monImportFixture.confirmations = []; window.confirm = message => { window.monImportFixture.confirmations.push(message); return false; }; });
  await page.keyboard.press("Escape");
  check(await modal().isVisible() && await count("cancel_audio_import") === 0 && await count("start_audio_import") === 0, "declined discard changed import");
  check(await modal().getByRole("textbox", { name: "会議名", exact: true }).inputValue() === "会議", "declined discard lost title");
  await page.evaluate(() => { window.confirm = message => { window.monImportFixture.confirmations.push(message); return true; }; });
  await page.keyboard.press("Escape"); await closed();
  check(await count("cancel_audio_import") === 1 && await page.evaluate(() => window.monImportFixture.confirmations.every(x => x.includes("確認内容を破棄"))), "cancel bypassed existing discard decision");
  results.push("Escape retains declined draft; explicit discard cancels same native selection once and restores opener");

  await boot(); await open(); await page.evaluate(() => { window.confirm = () => true; });
  const box = await modal().boundingBox(); await page.mouse.click(box.x - 10, box.y + 10); await closed();
  check(await count("start_audio_import") === 0 && await count("cancel_audio_import") === 1, "backdrop started import or failed cancel");
  results.push("backdrop uses existing token-bound cancellation without import admission");

  await boot(); await open();
  await page.evaluate(() => window.monImportFixture.changed({ phase: "waiting", busy: true }));
  await page.waitForFunction(() => document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled === true);
  await page.keyboard.press("Escape"); let bounds = await modal().boundingBox(); await page.mouse.click(bounds.x - 10, bounds.y + 10);
  check(await modal().isVisible() && await count("cancel_audio_import") === 0, "busy import dismissed before acknowledgement");
  await page.evaluate(() => window.monImportFixture.changed({ phase: "ready", busy: false }));
  await page.waitForFunction(() => !document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled);
  await page.evaluate(() => { window.confirm = () => true; });
  await modal().getByRole("button", { name: "確認をキャンセル", exact: true }).click(); await closed();
  results.push("shared native busy state blocks Escape, backdrop and common cancellation until completion");

  await boot(); await page.setViewportSize({ width: 360, height: 640 }); await open();
  bounds = await modal().boundingBox();
  check(bounds.x >= 23.9 && bounds.x + bounds.width <= 336.1 && await modal().evaluate(e => e.scrollWidth <= e.clientWidth + 1), "small import modal clipped or horizontally overflowed");
  await page.screenshot({ path: "import-modal-360.png" });
  await page.evaluate(() => { window.confirm = () => true; }); await page.keyboard.press("Escape"); await closed();
  results.push("production import fields and metadata fit a 360px window with 24px margins");

  await boot(); await page.setViewportSize({ width: 1200, height: 900 }); await open();
  await modal().getByRole("combobox", { name: "解析プロファイル", exact: true }).selectOption("high_accuracy");
  await page.evaluate(() => { window.monImportFixture.loseStart = true; window.monImportFixture.saveDelayMs = 800; });
  await modal().getByRole("button", { name: "取り込む", exact: true }).click();
  await page.keyboard.press("Escape"); check(await modal().isVisible(), "lost committed response closed before saved state");
  check(await modal().getByRole("button", { name: "確認をキャンセル", exact: true }).isDisabled(), "lost committed import dismissal enabled");
  await modal().waitFor({ state: "hidden" });
  check(await count("start_audio_import") === 1 && await page.evaluate(() => window.monImportFixture.meetings.length === 2 && window.monImportFixture.calls.find(c => c.command === "start_audio_import").args.request.options.profile === "high_accuracy"), "lost response duplicated import or substituted profile");
  results.push("lost start response waits for saved native receipt; single admission and retained high-accuracy choice");
  return { passed: results.length, results };
}
