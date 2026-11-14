async page => {
  await page.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, route => route.fulfill({contentType:"text/html",body:'<!doctype html><html lang="ja"><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module" src="/tests/data-save-browser-fixture.js"></script></body></html>'}));
  await page.goto("http://127.0.0.1:1420/");
  await page.getByRole("button", {name:"設定",exact:true}).click();
  const button = page.getByRole("button", {name:"バックアップを作成",exact:true});
  await page.waitForFunction(() => [...document.querySelectorAll('button')].some(button => button.textContent === "バックアップを作成" && !button.disabled));
  await page.evaluate(() => { window.dataFixture.blocker = "録音通知履歴の確認・更新中"; });
  for (let sample = 0; sample < 30; sample++) {
    if (!await button.isEnabled()) throw Error("notification polling disabled idle backup entry");
    await page.clock.runFor(100);
  }
  await page.evaluate(() => { window.dataFixture.blocker = "録音・音声保存中"; });
  await page.waitForFunction(() => [...document.querySelectorAll('button')].some(button => button.textContent === "バックアップを作成" && button.disabled));
  await page.evaluate(() => { window.dataFixture.blocker = null; });
  await page.waitForFunction(() => [...document.querySelectorAll('button')].some(button => button.textContent === "バックアップを作成" && !button.disabled));
  return {passed:2, results:["idle backup entry remains enabled across notification-history polling", "real recording/save activity still disables backup and completion restores it"]};
}
