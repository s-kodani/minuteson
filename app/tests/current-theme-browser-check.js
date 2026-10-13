// Production UI with simulated IPC only. No microphone, files or model execution.
async (page, capturePath = name => `/Users/skodani/Codebase/mon/output/playwright/current-registry-theme/${name}`) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const nav = page.getByRole("navigation", { name:"メインナビゲーション", exact:true });
  const settings = page.getByRole("region", { name:"表示設定", exact:true });
  const theme = settings.getByRole("combobox", { name:"テーマ", exact:true });
  check(await settings.getByRole("combobox").count() === 1, "display settings must contain theme only");
  check(await page.getByRole("combobox", {name:"本文の文字サイズ",exact:true}).count() === 0, "removed text-size control remains");
  check(await page.getByRole("checkbox", {name:"再生位置に追従",exact:true}).count() === 0, "removed follow control remains");
  check(await page.getByRole("banner").getByRole("combobox").count() === 0, "header theme selector returned");
  results.push("settings contain only theme and the header has no theme control");
  for (const value of ["light", "dark", "system"]) {
    await theme.selectOption(value);
    await page.waitForFunction(value => window.monBrowserFixture.uiPreferences?.payload.theme === value, value);
    check(await page.evaluate(() => {
      const p = window.monBrowserFixture.uiPreferences.payload;
      return p.schemaVersion === 1 && Object.keys(p).sort().join() === "schemaVersion,theme";
    }), "theme save retained removed display fields");
    if (value !== "system") await page.waitForFunction(value => document.documentElement.dataset.theme === value, value);
  }
  results.push("light dark and system save only the confirmed theme payload");
  for (const value of ["light", "dark"]) {
    await page.emulateMedia({colorScheme:value});
    await page.waitForFunction(value => document.documentElement.dataset.theme === value, value);
  }
  results.push("system theme follows simulated OS colors without writing preferences");
  const writes = await page.evaluate(() => window.monBrowserFixture.calls.filter(c=>c.command==="save_ui_preferences").length);
  await nav.getByRole("button", {name:"会議一覧",exact:true}).click();
  await nav.getByRole("button", {name:"設定",exact:true}).click();
  await theme.waitFor();
  check(await page.evaluate(() => window.monBrowserFixture.calls.filter(c=>c.command==="save_ui_preferences").length) === writes, "navigation added a theme save");
  results.push("returning to settings performs no implicit save");
  await page.screenshot({path:capturePath("theme-only-settings.png")});
  await nav.getByRole("button", {name:"会議一覧",exact:true}).click();
  await page.getByRole("button", {name:"会議A",exact:true}).click();
  await page.getByRole("region", {name:"文字起こし編集",exact:true}).waitFor();
  return results;
}
