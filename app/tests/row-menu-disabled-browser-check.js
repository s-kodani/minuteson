// The real workspace and deletion hooks, using synthetic IPC only.
async (page, capturePath, detailUI) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.emulateMedia({ colorScheme: "dark" });
  await page.evaluate(() => { const f=window.dataFixture; f.blocker = "AI処理中"; f.meetingBlockers = {[f.meetings[0].id]:[{kind:"job",reason:"AI処理中のため削除できません。",jobId:"synthetic-job"}]}; });
  await page.waitForFunction(() => document.documentElement.dataset.theme === "dark");
  await detailUI.openMeetingRow();
  const menu = page.getByRole("menu", { name: "既存会議の操作", exact: true });
  const remove = menu.getByRole("menuitem", { name: "既存会議を削除", exact: true });
  await page.waitForFunction(() => document.querySelector('.meeting-row-menu-note')?.textContent?.includes("AI処理中"));
  const audio = menu.getByRole("menuitem", { name: "既存会議の音声だけを削除", exact: true });
  check(await remove.isDisabled() && await audio.isDisabled(), "busy state enabled deletion");
  check(await menu.locator('.meeting-row-menu-note').count() === 1, "shared reason is repeated per button");
  for (const button of [remove, audio]) {
    const style = await button.evaluate(e => ({ background: getComputedStyle(e).backgroundColor, described: !!document.getElementById(e.getAttribute('aria-describedby')) }));
    check(style.background === "rgba(0, 0, 0, 0)" && style.described, "disabled action retains filled block or no accessible reason");
  }
  await page.screenshot({ path: capturePath("row-menu-disabled-dark.png") });
  const box = await remove.boundingBox(); await page.mouse.click(box.x + 10, box.y + box.height / 2);
  check(await page.evaluate(() => !window.dataFixture.calls.some(c => /review_meeting|start_data_operation/.test(c.command))), "disabled action sent a write or review");
  await detailUI.openMeetingRow();
  await page.keyboard.press("End");
  check(await menu.getByRole("menuitem", { name: "削除可否を再確認", exact: true }).evaluate(e => e === document.activeElement), "keyboard did not skip disabled items");
  await page.evaluate(() => { window.dataFixture.blocker = null; window.dataFixture.meetingBlockers = {}; });
  await page.waitForFunction(() => !document.querySelector('[role="menuitem"][aria-label="既存会議を削除"]').disabled);
  check(await menu.locator('.meeting-row-menu-note').count() === 0, "idle retains stale disabled reason");
  await audio.click();
  const dialog = page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: "音声だけの削除を確認", exact: true }) });
  await dialog.waitFor();
  check(await page.evaluate(() => window.dataFixture.calls.filter(c => c.command === 'review_meeting_audio_deletion').length === 1 && !window.dataFixture.calls.some(c => c.command === 'start_data_operation')), "idle review bypassed confirmation");
  await dialog.getByRole("button", { name: "音声を削除せず閉じる", exact: true }).click();
  return { passed: 4, results: ["actual AI blocker is visible and linked to both disabled actions", "disabled actions are neutral and transparent in dark mode", "pointer and keyboard cannot activate disabled deletion", "idle restores review and still requires deletion confirmation"] };
}
