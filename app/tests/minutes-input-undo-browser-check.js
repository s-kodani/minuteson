// Real editing keys against production UI; synthetic IPC, no native audio/data.
async page => {
  page.setDefaultTimeout(10000);
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const primary = () => page.getByRole("region", { name: "議事録生成と候補確認", exact: true });
  const field = () => page.getByRole("textbox", { name: "議事録ブロック2", exact: true });
  const boot = async (rich = false) => {
    await page.reload();
    await page.waitForFunction(() => !!window.__mon?.drafts?.a, null, { timeout: 10000 });
    if (rich) await page.evaluate(() => {
      const d = window.__mon.drafts.a.payload;
      d.blocks[2].items[0].content[0] = { type: "text", text: "点検日を確認する。\n", bold: true, italic: true };
      d.items[0].body = "点検日を確認する。\n";
    });
    await page.getByRole("button", { name: "議事録UI確認", exact: true }).click();
    await page.getByRole("tab", { name: "議事録", exact: true }).click();
    await primary().getByRole("button", { name: "議事録を編集", exact: true }).click();
  };
  const waitText = async text => {
    await page.waitForFunction(text => window.__mon.drafts.a.payload.blocks[1].content.map(n => n.text ?? "").join("") === text, text, { timeout: 10000 });
  };
  await boot();
  const original = await field().textContent();
  await field().click(); await field().press("End"); await field().press("Enter"); await waitText(original + "\n");
  await field().press("Meta+z"); await waitText(original);
  await field().press("Meta+Shift+z"); await waitText(original + "\n");
  await field().press("Enter"); await waitText(original + "\n\n");
  await field().press("Meta+z"); await waitText(original + "\n");
  // Plain-text paste remains an undoable native transaction; HTML stays text.
  await field().press("Meta+a");
  await field().evaluate(element => {
    const data = new DataTransfer(); data.setData("text/plain", "点検日\r\n<img src=x onerror=alert(1)>");
    data.setData("text/html", "<script>alert(1)</script>");
    element.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: data }));
  });
  await waitText("点検日\n<img src=x onerror=alert(1)>");
  check(await field().locator("img,script").count() === 0, "clipboard HTML became live DOM");
  await field().press("Meta+z"); await waitText(original + "\n");
  await field().press("Meta+Shift+z"); await waitText("点検日\n<img src=x onerror=alert(1)>");

  await boot(true);
  const rich = page.getByRole("textbox", { name: "議事録リスト3の項目1", exact: true });
  const baseline = await page.evaluate(() => structuredClone(window.__mon.drafts.a.payload));
  await rich.click({ position: { x: 8, y: 8 } }); await rich.press("Meta+a"); await rich.press("Backspace");
  await page.waitForFunction(() => window.__mon.drafts.a.payload.blocks[2].items[0].content.length === 0, null, { timeout: 10000 });
  await rich.press("Meta+z");
  await page.waitForFunction(content => JSON.stringify(window.__mon.drafts.a.payload.blocks[2].items[0].content) === JSON.stringify(content), baseline.blocks[2].items[0].content, { timeout: 10000 });
  await rich.press("Meta+Shift+z");
  await page.waitForFunction(() => window.__mon.drafts.a.payload.blocks[2].items[0].content.length === 0, null, { timeout: 10000 });
  // A real save error must leave native input history intact.
  await page.getByRole("button", { name: "議事録保存失敗を合成", exact: true }).click();
  await rich.click({ position: { x: 8, y: 8 } }); await rich.press("Meta+z");
  await page.waitForTimeout(1000);
  check(await rich.locator("strong em,em strong").count() === 1, "Undo lost rich formatting after a save error");
  check(await rich.locator("[data-evidence-id]").count() === 1, "Undo lost the same-owner evidence chip");
  await page.getByRole("button", { name: "議事録保存を復帰", exact: true }).click();
  await primary().getByRole("button", { name: "議事録保存を再試行", exact: true }).click();
  await page.waitForFunction(content => JSON.stringify(window.__mon.drafts.a.payload.blocks[2].items[0].content) === JSON.stringify(content), baseline.blocks[2].items[0].content, { timeout: 10000 });
  const result = await page.evaluate(() => ({ draft: window.__mon.drafts.a.payload, undo: window.__mon.calls.filter(c => c.cmd === "undo_minutes_edit").length }));
  check(JSON.stringify(result.draft.evidence.map(e => e.id)) === JSON.stringify(baseline.evidence.map(e => e.id)), "Undo reordered frozen references");
  for (const evidence of baseline.evidence) {
    const current = result.draft.evidence.find(e => e.id === evidence.id);
    check(!!current && ["id", "minutesItemId", "analysisRevisionId", "utteranceId", "quoteText", "startUs", "endUs"].every(key => current[key] === evidence[key]), "Undo rewrote a frozen reference");
  }
  check(result.undo === 0, "native input Undo called durable edit Undo");
  check(JSON.stringify(result.draft.blocks) === JSON.stringify(baseline.blocks), "Undo changed another block");
  return { passed: 6, results: ["newline Undo/Redo after autosave", "consecutive empty lines", "plain-text paste Undo/Redo and HTML rejection", "whole rich field deletion Undo/Redo", "save failure and retry retain native history", "same-owner frozen references restored; durable Undo unused"] };
}
