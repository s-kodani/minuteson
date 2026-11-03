async (page, capturePath, { approveModelLicense }) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  for (const target of ["macos", "windows"]) {
    const url = new URL(page.url()); url.search = `?target=${target}`;
    await page.goto(url.href);
    const selector = page.getByRole("combobox", { name: "文字起こしのモデル", exact: true });
    await selector.selectOption("official:synthetic-catalog");
    const row = page.locator(".model-selection-row").filter({ has: selector });
    await row.getByRole("button", { name: "使用", exact: true }).click();
  await approveModelLicense();
    await row.getByRole("button", { name: "使用中", exact: true }).waitFor();
    const state = await page.evaluate(() => ({ calls: window.modelDownloadFixture.calls, entries: window.modelDownloadFixture.entries, profiles: window.modelDownloadFixture.profiles }));
    check(state.entries.length === 1 && state.profiles[0].components[0].packageId === "44444444-4444-4444-8444-444444444444", "replaced the existing package");
    check(!state.calls.some(call => ["start_model_download", "place_downloaded_model"].includes(call.command)), "unchanged package was acquired again");
    check(state.calls.filter(call => call.command === "check_model_compatibility").length === 1, "failed preparation did not receive one new check");
    results.push(`${target}: changed transport and failed prior check reuse the same package without download/copy`);
  }
  return { passed: results.length, results };
}
