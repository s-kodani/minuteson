async (page, capturePath, detailUI) => {
  const results = [], check = (condition, message) => { if (!condition) throw Error(message); };
  try {
  await detailUI.openMeetingRow();
  const trigger = page.getByRole("button", { name: "既存会議の操作", exact: true });
  const menu = page.getByRole("menu", { name: "既存会議の操作", exact: true });
  await menu.getByRole("menuitem", { name: "既存会議の音声だけを削除", exact: true }).click();
  const modal = page.getByRole("dialog", { name: "音声だけを削除", exact: true });
  await modal.waitFor();
  check(await modal.getByRole("button").count() === 2 && await modal.getByRole("checkbox").count() === 0, "simple confirmation retained extra consent controls");
  check(await menu.isHidden() && await page.evaluate(() => !!document.activeElement?.closest("dialog[open]")), "popup stayed above the confirmation or took focus");
  check(await page.evaluate(() => !window.dataFixture.calls.some(call => call.command === "start_data_operation")), "opening confirmation deleted audio");
  results.push("menu action opens the existing confirmation and hides the popup without deletion");
  await page.keyboard.press("Escape"); await modal.waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "既存会議の操作");
  check(await trigger.evaluate(element => element === document.activeElement), "confirmation returned to the hidden menu item instead of the ellipsis");
  check(await page.evaluate(() => !window.dataFixture.calls.some(call => call.command === "start_data_operation")), "cancelling confirmation changed data");
  results.push("cancelling returns focus to the stable ellipsis without changing any audio or meeting");
  await page.getByRole("button",{name:"既存会議",exact:true}).click();
  check(await page.getByRole("alert").count() === 0, "opening meeting detail emitted unrelated fixture errors");
  const detailTrigger=page.getByRole("button",{name:"会議の操作",exact:true});await detailTrigger.click();
  const detailMenu=page.getByRole("menu",{name:"会議の操作",exact:true});
  await detailMenu.getByRole("menuitem",{name:"既存会議の音声だけを削除",exact:true}).click();await modal.waitFor();
  await page.keyboard.press("Escape");await modal.waitFor({state:"hidden"});
  check(await page.getByRole("tab",{name:"文字起こし",exact:true}).isVisible(),"audio cancellation left detail");
  await detailTrigger.click();await detailMenu.getByRole("menuitem",{name:"既存会議の音声だけを削除",exact:true}).click();await modal.waitFor();
  await modal.getByRole("button",{name:"削除",exact:true}).click();
  await page.waitForFunction(()=>window.audioDeletionFixture.deleted===1);await modal.waitFor({state:"hidden"});
  check(await page.getByRole("tab",{name:"文字起こし",exact:true}).isVisible()&&await page.locator(".meeting-title-row h1").innerText()==="既存会議","audio deletion left the meeting detail");
  check(await page.evaluate(()=>window.dataFixture.calls.filter(c=>c.command==="start_data_operation").length)===1,"audio deletion sent duplicate requests");
  results.push("detail audio deletion preserves the meeting detail after cancellation and confirmed deletion");
  return { passed: results.length, results };
  } catch (cause) { throw Error(`${cause}; body=${await page.locator('body').innerText()}; calls=${JSON.stringify(await page.evaluate(() => window.dataFixture.calls.filter(c => /review|start_|stop_|flush/u.test(c.command))))}`); }
}
