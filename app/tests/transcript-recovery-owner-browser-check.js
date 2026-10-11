// Presentation regression using visible controls; never deletes recovery data.
async (page) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const entry = page.getByRole("button", { name: "回復入力・話者・一括編集操作", exact: true });
  const inventoryEntry = page.getByRole("button", { name: "旧下書きを含む回復記録", exact: true });
  const close = name => page.getByRole("dialog", { name, exact: true }).getByRole("button", { name: "確認を閉じる", exact: true }).press("Escape");
  await page.locator(".meeting-name").first().click();
  for (let i = 0; i < 2; i += 1) {
    await entry.click();
    await inventoryEntry.click();
    const inventory = page.getByRole("dialog", { name: "未確定入力の回復記録", exact: true });
    await inventory.waitFor();
    check(await page.locator("dialog[open]").count() === 1, "recovery opened another native boundary");
    await inventory.getByRole("button", { name: "回復一覧を再読込", exact: true }).click();
    await inventory.getByText("回復記録はありません。", { exact: true }).waitFor();
    await close("未確定入力の回復記録");
    await page.getByRole("dialog", { name: "文字起こしの編集オプション", exact: true }).waitFor();
    check(await inventoryEntry.evaluate(e => e === document.activeElement), "inventory dismissal lost its entry");
    await close("文字起こしの編集オプション");
    check(await page.locator("dialog[open]").count() === 0, "edit dismissal left a dialog open");
    check(await entry.evaluate(e => e === document.activeElement), "edit dismissal lost its workspace entry");
  }
  return { passed: 1, results: ["recovery shares one boundary and returns to both entry points twice"] };
}
