// Real React UI with local fixture settings. No native window or system changes.
async (page, capturePath) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const host = page.getByRole("complementary", { name: "通知", exact: true });
  const select = page.getByRole("combobox", { name: "表示テーマ", exact: true });
  const count = () => page.evaluate(() => window.themeFixture.calls.filter(c => c.command === "save_ui_preferences").length);
  await page.evaluate(() => window.themeFixture.reset()); await page.reload(); await select.waitFor();
  await page.waitForFunction(() => !document.querySelector(".theme-quick select")?.disabled);
  await page.waitForTimeout(2700);
  check(await host.locator(".toast-card").count() === 0, "routine theme read created notification");
  await select.selectOption("dark"); await host.getByRole("status").filter({ hasText: "表示設定を保存しました" }).waitFor();
  const first = await host.locator(".toast-success").getAttribute("data-toast-id");
  await host.getByRole("button", { name: "成功通知を閉じる", exact: true }).click();
  await page.getByRole("button", { name: "設定", exact: true }).click();
  const settings = page.getByRole("region", { name: "表示設定", exact: true });
  check(await host.locator(".toast-card").count() === 0, "navigation resurrected acknowledged result");
  await select.selectOption("light"); await host.getByRole("status").filter({ hasText: "表示設定を保存しました" }).waitFor();
  check(await host.locator(".toast-success").getAttribute("data-toast-id") !== first, "another save with same wording was suppressed");
  results.push("periodic reads are silent; explicit saves notify once per operation, dismissal and view changes do not replay, next save has new identity");

  await page.evaluate(() => { window.themeFixture.mode = "fail"; }); await select.selectOption("dark");
  await host.getByRole("alert").filter({ hasText: "表示設定の読込・保存" }).waitFor();
  const failed = await count(); await host.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await settings.getByLabel("表示設定の読込・保存の要対応状態", { exact: true }).isVisible() && await select.inputValue() === "dark" && await page.evaluate(() => document.documentElement.dataset.theme) === "light", "dismissal reset requested theme, confirmed colors or issue");
  await page.waitForTimeout(2700); check(await count() === failed && await host.locator(".toast-error").count() === 0, "polling resubmitted save or resurrected same failure");
  await page.evaluate(() => { window.themeFixture.mode = "success"; }); await settings.getByRole("button", { name: "テーマの保存を再試行", exact: true }).click();
  await settings.getByLabel("表示設定の読込・保存の要対応状態", { exact: true }).waitFor({ state: "hidden" });
  check(await count() === failed + 1, "explicit retry wrote more than once");
  results.push("important theme failure dismissal keeps choice, saved colors, issue and inline retry; polls neither replay nor rewrite; explicit retry resolves");

  await page.evaluate(() => { window.themeFixture.mode = "fail"; }); await select.selectOption("light");
  await host.getByRole("button", { name: "テーマの保存を再試行", exact: true }).waitFor();
  await page.evaluate(() => { window.themeFixture.mode = "success"; }); await host.getByRole("button", { name: "テーマの保存を再試行", exact: true }).click();
  await host.locator(".toast-error").waitFor({ state: "hidden" });
  check(await select.inputValue() === "light", "toast retry changed the retained selection");
  results.push("safe theme toast retry persists retained selection through the same guarded controller");

  await page.evaluate(() => { window.themeFixture.mode = "lostUnreadable"; }); await select.selectOption("dark");
  await host.getByRole("alert").filter({ hasText: "保存結果を確認できません" }).waitFor();
  const uncertain = await count();
  check(await select.isDisabled() && await host.getByRole("button", { name: "表示設定の結果を確認", exact: true }).isEnabled(), "unknown save permits another write or has no read confirmation");
  await page.evaluate(() => { window.themeFixture.readFail = false; }); await host.getByRole("button", { name: "表示設定の結果を確認", exact: true }).click();
  await host.getByRole("status").filter({ hasText: "保存済みの表示設定を確認しました" }).waitFor();
  check(await count() === uncertain && await select.inputValue() === "dark", "unknown committed save was repeated");
  results.push("unknown save offers authoritative read, not another save; confirmed commit emits information and keeps write count unchanged");

  await host.getByRole("button", { name: "情報通知を閉じる", exact: true }).click();
  await page.evaluate(() => { window.themeFixture.mode = "success"; });
  await settings.getByRole("button", { name: "保存済み表示設定を再読込", exact: true }).click();
  await host.getByRole("status").filter({ hasText: "再読込しました" }).waitFor();
  await page.screenshot({ path: "theme-toast-dark.png" });
  results.push("explicit read refresh notifies as information without a success banner or write");

  const mini = await page.context().newPage();
  try {
    await mini.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => { const response = await route.fetch(); await route.fulfill({ response, body: (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/theme-browser-fixture.js");</script>') }); });
    await mini.goto("http://127.0.0.1:1420/?window=recorder"); await mini.waitForFunction(() => !!window.themeFixture);
    await mini.evaluate(() => { window.themeFixture.readFail = true; }); await mini.getByRole("alert").filter({ hasText: "表示設定を確認できません" }).waitFor();
    check(await mini.locator(".toast-host").count() === 0 && await mini.evaluate(() => window.themeFixture.calls.filter(c => c.command === "save_ui_preferences").length) === 0, "mini hid its issue in absent host or wrote preferences");
    await mini.setViewportSize({ width: 380, height: 130 }); await mini.screenshot({ path: capturePath("mini-theme-read-error.png") });
    results.push("recorder retains its existing local read error with no full-size duplicate toast or preference write; native routing remains pending");
  } finally { await mini.close(); }
  return { passed: results.length, results };
}
