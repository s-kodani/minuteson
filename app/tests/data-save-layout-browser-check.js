// Real browser layout and cancellation; all IPC and file selection are synthetic.
async (page, capturePath) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  await page.getByRole("button", { name: "設定", exact: true }).click();
  const region = page.getByRole("region", { name: "データと保存", exact: true });
  const row = region.locator('.data-save-primary-actions');
  const move = row.getByRole("button", { name: "会議保存先を変更", exact: true });
  await move.waitFor();
  check(await row.getByRole("button").allTextContents().then(labels => JSON.stringify(labels) === JSON.stringify(["バックアップを作成", "バックアップから追加復元", "会議保存先を変更"])), "primary button order changed");
  check(await region.getByRole("button", { name: "状態を再確認", exact: true }).count() === 0, "manual refresh remains");
  for (const width of [1280, 960]) {
    await page.setViewportSize({ width, height: 900 });
    const boxes = await row.getByRole("button").evaluateAll(buttons => buttons.map(button => {
      const rect = button.getBoundingClientRect(), style = getComputedStyle(button);
      return { left: rect.left, right: rect.right, top: rect.top, height: rect.height, font: style.fontSize, line: style.lineHeight };
    }));
    check(boxes.every(box => Math.abs(box.top - boxes[0].top) <= 1 && Math.abs(box.height - boxes[0].height) <= 1 && box.font === boxes[0].font && box.line === boxes[0].line), `button metrics differ at ${width}`);
    check(boxes[2].left - boxes[1].right >= 8 && boxes[1].left - boxes[0].right >= 8, `button spacing missing at ${width}`);
    check(boxes[2].right <= width && await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `horizontal overflow at ${width}`);
    results.push(`three aligned, spaced actions without manual refresh at ${width}px`);
  }
  await page.screenshot({ path: capturePath("settings-data-actions.png") });
  await move.click();
  const modal = page.getByRole("dialog", { name: "会議保存先の変更確認", exact: true });
  await modal.waitFor();
  check(await page.evaluate(() => !window.dataFixture.calls.some(call => call.command === "start_data_operation")), "moving published without consent");
  await page.keyboard.press("Escape"); await modal.waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "会議保存先を変更");
  check(await move.evaluate(button => button === document.activeElement) && await page.evaluate(() => !window.dataFixture.calls.some(call => call.command === "start_data_operation")), "cancellation lost focus or changed data");
  results.push("move confirmation cancels without writes and returns focus to its relocated button");
  return { passed: results.length, results };
}
