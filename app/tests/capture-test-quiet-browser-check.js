// Production UI with synthetic capture ownership, without native recording.
async page => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.unrouteAll({ behavior: "wait" });
  await page.route("http://127.0.0.1:1420/", async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()).replace("/src/bootstrap.tsx", "/tests/application-close-browser-fixture.js") });
  });
  await page.goto("http://127.0.0.1:1420/");
  await page.getByRole("button", { name: "新しい録音", exact: true }).click();
  const panel = page.locator(".capture-test-dialog");
  await page.getByRole("button", { name: "5秒録音して確認", exact: true }).click();
  await page.getByRole("dialog", { name: "5秒の録音前テストを開始しますか？", exact: true })
    .getByRole("button", { name: "5秒テストを開始する", exact: true }).click();
  await panel.getByRole("button", { name: "テストを取り消す", exact: true }).click();
  await panel.getByText("テストの停止完了を待っています", { exact: true }).waitFor();
  await page.evaluate(() => window.closeFixture.testChanged({ phase: "cancelled", busy: false, errorMessage: null }));
  await panel.getByText("テストを取り消しました", { exact: true }).waitFor();
  check(await page.locator(".toast-card").count() === 0, "normal cancellation emitted a toast");
  await panel.getByRole("button", { name: "もう一度5秒録音して確認", exact: true }).click();
  await page.getByRole("dialog", { name: "5秒の録音前テストを開始しますか？", exact: true })
    .getByRole("button", { name: "5秒テストを開始する", exact: true }).click();
  await page.evaluate(() => window.closeFixture.testReady());
  await panel.getByText("停止と音声の検証が完了しました", { exact: true }).waitFor();
  check(await page.locator(".toast-card").count() === 0, "successful test emitted a toast");
  await page.evaluate(() => { window.closeFixture.testDiscardFailure = true; });
  await panel.getByRole("button", { name: "テスト音声を破棄", exact: true }).click();
  await panel.getByRole("alert").filter({ hasText: "模擬テスト回収未完了" }).waitFor();
  check(await page.locator(".toast-card").count() === 0, "discard failure emitted a toast");
  await panel.getByRole("button", { name: "確認を閉じる", exact: true }).click();
  await page.locator(".capture-input-hint").filter({ hasText: "模擬テスト回収未完了" }).waitFor();
  await page.getByRole("button", { name: "テスト結果を確認", exact: true }).click();
  await page.evaluate(() => { window.closeFixture.testDiscardFailure = false; });
  await panel.getByRole("button", { name: "テスト音声を破棄", exact: true }).click();
  await panel.getByRole("button", { name: "5秒録音して確認", exact: true }).click();
  await page.getByRole("dialog", { name: "5秒の録音前テストを開始しますか？", exact: true })
    .getByRole("button", { name: "5秒テストを開始する", exact: true }).click();
  await page.evaluate(() => window.closeFixture.testChanged({ phase: "error", busy: false, cleanupPending: true, errorMessage: "模擬の停止失敗" }));
  await panel.getByRole("alert").filter({ hasText: "模擬の停止失敗" }).waitFor();
  await panel.getByRole("alert").filter({ hasText: "一時音声の回収が未完了" }).waitFor();
  check(await page.locator(".toast-card").count() === 0, "capture failure emitted a toast");
  await page.reload();
  await page.getByRole("button", { name: "新しい録音", exact: true }).click();
  await page.evaluate(() => { window.closeFixture.testLost = true; });
  await page.getByRole("button", { name: "5秒録音して確認", exact: true }).click();
  const confirmation = page.getByRole("dialog", { name: "5秒の録音前テストを開始しますか？", exact: true });
  await confirmation.getByRole("button", { name: "5秒テストを開始する", exact: true }).click();
  await confirmation.getByRole("alert").filter({ hasText: "模擬テスト開始応答喪失" }).waitFor();
  check(await page.locator(".toast-card").count() === 0, "start confirmation failure emitted a toast");
  return "PASS: cancellation, success, discard/capture/cleanup/start confirmation failures stay inline; closed panel retains error hint";
}
