async (page, capturePath) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.goto("http://127.0.0.1:1420/tests/error-notifications-browser.html");
  await page.getByRole("button", {name:"編集を開く",exact:true}).click();
  const modal = page.getByRole("dialog", {name:"編集の検証",exact:true});
  const notifications = () => page.getByRole("complementary", {name:"通知",exact:true});
  await modal.getByRole("button", {name:"保存失敗を発生",exact:true}).click();
  await notifications().getByRole("alert").filter({hasText:"合成の保存失敗"}).waitFor();
  if (capturePath) await page.screenshot({path:capturePath("recording-error-notification-ui.png")});
  check(await modal.evaluate(element => [...element.querySelectorAll('[role="alert"]')].every(node => node.closest('.toast-host'))), "error rendered in modal body");
  check(await modal.getByRole("textbox").inputValue() === "保持する入力", "failure lost edits");
  await modal.getByRole("button", {name:"保存失敗を発生",exact:true}).click();
  check(await notifications().getByRole("alert").count() === 1, "same failure duplicated");
  await notifications().getByRole("button", {name:"エラー通知を閉じる",exact:true}).click();
  check(await modal.getByRole("textbox").inputValue() === "保持する入力", "dismissal changed edits");
  await modal.getByRole("button", {name:"再試行を成功",exact:true}).click();
  await notifications().getByRole("alert").waitFor({state:"hidden"});
  await modal.getByRole("button", {name:"保存失敗を発生",exact:true}).click();
  await notifications().getByRole("alert").waitFor();
  await modal.getByRole("button", {name:"再試行を成功",exact:true}).click();
  await notifications().getByRole("alert").waitFor({state:"hidden"});
  await modal.getByRole("button", {name:"確認を閉じる",exact:true}).click();
  await page.getByRole("button", {name:"通知履歴",exact:true}).click();
  await page.getByRole("region", {name:"通知一覧の内容",exact:true}).getByText(/合成の保存失敗/).first().waitFor();
  return {passed:5, results:["modal failures render in notifications only", "input survives failure", "repeated failure deduplicates", "dismissal and retry preserve edits and clear active failure", "history retains the diagnostic"]};
}
