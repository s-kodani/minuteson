// CLI-first real browser events with synthetic IPC. No real meeting deletion.
async (page, capturePath, detailUI) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const calls = command => page.evaluate(name => window.dataFixture.calls.filter(call => call.command === name), command);
  const trigger = () => page.getByRole("button", { name: "既存会議の操作", exact: true });
  const menu = () => page.getByRole("menu", { name: "既存会議の操作", exact: true });
  const open = () => menu().getByRole("menuitem", { name: "開く", exact: true });
  const remove = () => menu().getByRole("menuitem", { name: "既存会議を削除", exact: true });
  const dialog = () => page.getByRole("dialog", { name: "会議を削除", exact: true });
  const back = async () => { await page.getByRole("navigation", { name: "画面の移動" }).getByRole("button", { name: "前の画面へ戻る", exact: true }).click(); await trigger().waitFor(); };
  const show = async () => { await detailUI.openMeetingRow(); await menu().waitFor(); await page.waitForFunction(() => !document.querySelector('[role="menuitem"][aria-label="既存会議を削除"]').disabled); };

  await show(); await open().click();
  await page.getByRole("tab", { name: "文字起こし", exact: true }).waitFor();
  const targetId = await page.evaluate(() => window.dataFixture.meetings[0].id);
  check((await calls("get_meeting")).some(call => call.args.id === targetId), "pointer click did not open the selected meeting");
  check(await page.locator('.toast-card [role="alert"]').count() === 0, "opening the selected meeting produced an operation error");
  await back();
  results.push("pointer click reaches the selected meeting instead of dismissing before click");

  await show(); const before = (await calls("get_meeting")).length;
  const bounds = await open().boundingBox();
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2); await page.mouse.down();
  check(await menu().isVisible() && await open().evaluate(element => element === document.activeElement), "mouse-down moved focus behind the popup");
  check((await calls("get_meeting")).length === before && (await calls("review_meeting_deletion")).length === 0, "mouse-down executed an action");
  await page.mouse.move(400, 100); await page.mouse.up();
  check((await calls("get_meeting")).length === before && (await calls("review_meeting_deletion")).length === 0, "release outside executed the pressed item");
  await page.keyboard.press("Escape"); await menu().waitFor({ state: "hidden" });
  results.push("press keeps item focus but neither press nor release outside activates an operation");

  for (const key of ["Enter", "Space"]) {
    await show(); await page.keyboard.press(key);
    await page.getByRole("tab", { name: "文字起こし", exact: true }).waitFor(); await back();
    results.push(`${key} activation opens the meeting and returns to its list`);
  }

  await show(); await remove().click(); await dialog().waitFor(); await dialog().getByText("削除対象フォルダー", { exact: true }).waitFor();
  check(await menu().isHidden() && (await calls("review_meeting_deletion")).length === 1 && (await calls("start_data_operation")).length === 0, "delete item bypassed review or failed to hide the popup");
  check(await dialog().getByText("削除対象フォルダー", {exact:true}).isVisible(), "deletion target directory is missing");
  await dialog().getByRole("button", { name: "キャンセル", exact: true }).click(); await dialog().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "既存会議の操作");
  check((await calls("start_data_operation")).length === 0 && await page.evaluate(() => window.deletionFixture.deleted) === 0, "cancelling deleted synthetic data");
  results.push("delete opens one confirmation and cancellation restores the ellipsis without deleting");

  await show(); await open().click();
  const detailTrigger=page.getByRole("button",{name:"会議の操作",exact:true});
  await detailTrigger.click();
  const detailMenu=page.getByRole("menu",{name:"会議の操作",exact:true});
  await detailMenu.getByRole("menuitem",{name:"既存会議を削除",exact:true}).click();await dialog().waitFor();
  await dialog().getByRole("button",{name:"キャンセル",exact:true}).click();await dialog().waitFor({state:"hidden"});
  check(await page.getByRole("tab",{name:"文字起こし",exact:true}).isVisible()&&await detailTrigger.evaluate(e=>e===document.activeElement),"detail delete cancellation left the meeting or lost its opener");
  results.push("detail delete cancellation preserves the meeting and restores its popup trigger");
  await detailTrigger.click();await detailMenu.getByRole("menuitem",{name:"既存会議を削除",exact:true}).click();await dialog().waitFor();
  await dialog().getByText("削除対象フォルダー", { exact: true }).waitFor();
  await page.screenshot({ path: capturePath("row-menu-delete-confirmation.png") });
  await dialog().getByRole("button", { name: "削除", exact: true }).click();
  await page.waitForFunction(() => window.deletionFixture.deleted === 1 && window.dataFixture.meetings.length === 1);
  await trigger().waitFor({ state: "hidden" });
  check((await calls("start_data_operation")).length === 1 && await page.getByRole("button", { name: "残す会議の操作", exact: true }).isVisible(), "confirmed deletion repeated or removed another meeting");
    await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();
  check(await page.getByRole("tab",{name:"文字起こし",exact:true}).count()===0,"deleted meeting detail stayed open");
  results.push("detail delete executes once, removes only the target and returns to the meeting list");
  return { passed: results.length, results };
}
