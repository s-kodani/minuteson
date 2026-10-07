// Production editor and notification host, simulated IPC only.
async (page, capturePath, detailUI) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const host = page.getByRole("complementary", { name: "通知", exact: true });
  const vocabularyNotice = host.locator(".toast-card").filter({ hasText: "用語の読込・保存" });
  const writes = () => page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command === "save_vocabulary_book"));
  for (const scope of ["global", "meeting-0"]) {
    await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(key => { window.monBrowserFixture.vocabulary[key] = { id: key, lockVersion: 0, payload: [{ id: "term", term: "点検日", reading: null, note: null, enabled: true }] }; }, scope);
    let panel;
    if (scope === "global") { await page.getByRole("button", { name: "設定", exact: true }).click(); panel = page.getByRole("region", { name: "共通用語", exact: true }); }
    else { await page.getByRole("button", { name: "開く", exact: true }).first().click(); await detailUI.openMeetingTools(); panel = page.locator("section").filter({ has: page.getByRole("heading", { name: "会議の用語", exact: true }) }).last(); await panel.getByText("会議別用語（任意）", { exact: true }).click(); }
    const term = panel.getByRole("textbox", { name: "用語（必須）", exact: true }); await term.waitFor();
    await term.fill("点検予定日"); await panel.getByRole("status").filter({ hasText: "用語は保存済み" }).waitFor();
    check(await host.locator(".toast-success").count() === 0, "routine vocabulary autosave generated success toast");
    results.push(`${scope}: normal autosave remains short inline state without success spam`);

    await page.evaluate(() => { window.monBrowserFixture.failVocabulary = true; }); await term.fill("失敗した点検日");
    await host.getByRole("alert").filter({ hasText: "用語の読込・保存" }).waitFor();
    const before = (await writes()).length;
    if (scope !== "global") await panel.getByText("会議別用語（任意）", { exact: true }).click();
    check(await panel.getByLabel("用語の読込・保存の要対応状態", { exact: true }).isVisible(), "collapsed optional editor hid failed save state");
    await vocabularyNotice.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
    check((await writes()).length === before, "closing failure notification wrote vocabulary");
    if (scope !== "global") await panel.getByText("会議別用語（任意）", { exact: true }).click();
    check(await term.inputValue() === "失敗した点検日" && await panel.getByRole("button", { name: "用語保存を再試行", exact: true }).isEnabled(), "dismissal lost failed input or explicit retry");
    results.push(`${scope}: persistent failure, short retained issue even collapsed, dismissal preserves input and retry without writes`);

    await term.fill(""); check(await panel.getByRole("button", { name: "用語保存を再試行", exact: true }).isDisabled(), "invalid vocabulary allowed save retry");
    await term.fill("修正した点検日"); await page.evaluate(() => { window.monBrowserFixture.failVocabulary = false; });
    await panel.getByRole("button", { name: "用語保存を再試行", exact: true }).click();
    await panel.getByRole("status").filter({ hasText: "用語は保存済み" }).waitFor();
    await panel.getByLabel("用語の読込・保存の要対応状態", { exact: true }).waitFor({ state: "hidden" });
    const saved = await writes();
    check(saved.length === before + 1 && saved.at(-1).args.meetingId === (scope === "global" ? null : scope) && saved.at(-1).args.payload[0].term === "修正した点検日", "retry saved wrong scope, invalid terms or duplicate write");
    results.push(`${scope}: invalid input disables retry; explicit retry persists only valid latest input in the exact book`);

    await page.evaluate(() => { window.monBrowserFixture.failVocabulary = true; }); await term.fill("再度失敗した点検日");
    const retry = host.getByRole("button", { name: "用語保存を再試行", exact: true }); await retry.waitFor();
    await panel.getByRole("button", { name: "用語を再読込", exact: true }).click();
    const modal = page.getByRole("dialog", { name: "用語入力の破棄を確認", exact: true }); await modal.waitFor();
    const paused = (await writes()).length;
    await page.waitForTimeout(850); check((await writes()).length === paused, "pending discard resubmitted failed save");
    await page.keyboard.press("Escape"); await modal.waitFor({ state: "hidden" });
    check(await term.inputValue() === "再度失敗した点検日", "cancelled discard lost input");
    await page.evaluate(() => { window.monBrowserFixture.failVocabulary = false; window.monBrowserFixture.persistenceDelayMs = 250; });
    await retry.click(); await panel.getByRole("status").filter({ hasText: "用語は保存済み" }).waitFor();
    check((await writes()).length === paused + 1 && await term.inputValue() === "再度失敗した点検日", "toast retry bypassed queue or reset input");
    results.push(`${scope}: discard confirmation pauses saving; cancel keeps edits, guarded toast retry uses existing queue once`);
  }
  return { passed: results.length, results };
}
