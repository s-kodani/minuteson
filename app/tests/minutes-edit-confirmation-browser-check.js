// Production rich editor with minutes-browser-fixture.js; no real documents.
async (page, capturePath, detailUI) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const editor = () => page.getByRole("region", { name: "議事録文書と編集", exact: true });
  const modal = title => page.getByRole("dialog", { name: title, exact: true });
  const count = command => page.evaluate(c => window.__mon.calls.filter(x => x.cmd === c).length, command);
  const boot = async () => { await page.reload(); await page.getByRole("button", { name: "開く", exact: true }).first().click(); await detailUI.openMinutes(); await editor().getByRole("button", { name: "議事録を編集", exact: true }).click(); await editor().getByRole("textbox", { name: "議事録ブロック2", exact: true }).waitFor(); };
  await boot(); const text = editor().getByRole("textbox", { name: "議事録ブロック2", exact: true });
  const original = await text.textContent(); await text.fill("未保存の点検日メモ");
  await editor().getByRole("button", { name: "議事録編集を再読込", exact: true }).click();
  const discard = "議事録編集の破棄を確認"; await modal(discard).waitFor(); await page.waitForTimeout(1000);
  check(await count("save_minutes_draft") === 0, "rich autosave ran during discard choice");
  await page.keyboard.press("Escape"); await modal(discard).waitFor({ state: "hidden" });
  check(await text.textContent() === "未保存の点検日メモ", "cancel discarded rich input");
  await editor().getByRole("button", { name: "議事録編集を再読込", exact: true }).click();
  await modal(discard).getByRole("button", { name: "入力を破棄して再読込する", exact: true }).click(); await modal(discard).waitFor({ state: "hidden" });
  check(await text.textContent() === original && await count("save_minutes_draft") === 0, "discard saved unwanted rich input");
  results.push("rich discard suspends autosave; cancel keeps input; explicit reload restores only saved document");

  await boot(); const originalDraft = await page.evaluate(() => JSON.stringify(window.__mon.drafts.a.payload));
  const generated = await page.evaluate(() => JSON.stringify(window.__mon.views.a.content));
  const item = editor().getByRole("textbox", { name: "議事録リスト3の項目1", exact: true });
  await editor().getByText("議事録の書式・構造操作", { exact:true }).click(); await item.click(); const remove = editor().getByRole("button", { name: "ブロックを削除", exact: true }); await remove.click();
  const deletion = "議事録ブロックの削除を確認"; await modal(deletion).waitFor();
  check((await modal(deletion).innerText()).includes("項目・根拠") && await count("save_minutes_draft") === 0, "delete scope omitted or started before consent");
  await page.keyboard.press("Escape"); await modal(deletion).waitFor({ state: "hidden" });
  check(await page.evaluate(() => JSON.stringify(window.__mon.drafts.a.payload)) === originalDraft, "cancel deleted saved block");
  await item.click(); await remove.click(); await page.evaluate(() => window.__mon.minutesSaveDelay = 500);
  await modal(deletion).getByRole("button", { name: "ブロックを削除する", exact: true }).click();
  await page.keyboard.press("Escape"); check(await modal(deletion).isVisible(), "delete closed before durable acknowledgement");
  await modal(deletion).waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.classList.contains("minutes-rich-field"));
  check(await count("save_minutes_draft") === 1 && await page.evaluate(() => !window.__mon.drafts.a.payload.items.some(i => i.id === "item-a") && !window.__mon.drafts.a.payload.evidence.some(e => e.minutesItemId === "item-a")) && await page.evaluate(() => JSON.stringify(window.__mon.views.a.content)) === generated, "deletion changed generated original or left orphan item/evidence");
  await editor().getByRole("button", { name: "直前の議事録編集を取り消す", exact: true }).click();
  await page.waitForFunction(v => JSON.stringify(window.__mon.drafts.a.payload) === v, originalDraft);
  results.push("explicit block deletion waits for durable save, removes owned items/evidence only, focuses remaining field and is undone exactly");

  await boot(); await editor().getByText("議事録の書式・構造操作", { exact:true }).click(); await item.click(); await remove.click(); await page.evaluate(() => window.__mon.failMinutesSave = true);
  await modal(deletion).getByRole("button", { name: "ブロックを削除する", exact: true }).click(); await modal(deletion).getByRole("alert").waitFor();
  check(await count("save_minutes_draft") === 1 && await page.evaluate(n => window.__mon.drafts.a.payload.blocks.length === n, JSON.parse(originalDraft).blocks.length), "failed deletion claimed native save or replayed");
  await modal(deletion).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  await page.evaluate(() => window.__mon.failMinutesSave = false);
  await editor().getByRole("button", { name: "議事録保存を再試行", exact: true }).click();
  await page.waitForFunction(n => window.__mon.drafts.a.payload.blocks.length === n - 1, JSON.parse(originalDraft).blocks.length);
  check(await count("save_minutes_draft") === 2, "explicit rich save retry repeated queued deletion");
  results.push("failed block save retains exact queued deletion and original generated document; explicit queue retry saves once");
  return { passed: results.length, results };
}
