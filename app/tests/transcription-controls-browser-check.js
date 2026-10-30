// Production UI with synthetic IPC; no recording or inference is started.
async page => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.unrouteAll({ behavior: "wait" });
  await page.route("http://127.0.0.1:1420/", async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()).replace("/src/bootstrap.tsx", "/tests/browser-fixture.js") });
  });
  await page.goto("http://127.0.0.1:1420/");
  await page.getByRole("button", { name: "会議A", exact: true }).click();
  await page.getByRole("button", { name: "文字起こしをコピー", exact: true }).waitFor();
  await page.evaluate(() => {
    const fixture = window.monBrowserFixture;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { write: async ([item]) => {
      const blob = await item.getType("text/plain"); fixture.copiedText = await blob.text();
    } } });
    fixture.persistenceDelayMs = 50;
  });
  await page.getByRole("button", { name: "00:00:00の発言を編集", exact: true }).click();
  await page.getByRole("textbox", { name: "00:00:00の発言", exact: true }).fill("コピーする編集本文");
  await page.getByRole("button", { name: "文字起こしをコピー", exact: true }).click();
  await page.getByRole("complementary", {name:"通知",exact:true}).getByRole("status").filter({hasText:"文字起こしをコピーしました。"}).waitFor();
  check(await page.locator(".transcript-copy-status").count() === 0, "copy status remains beside button");
  check(await page.evaluate(() => window.monBrowserFixture.copiedText === "[00:00:00] 田中：コピーする編集本文\n[00:00:20] 田中、佐藤：はい、確認します。"), "copy omitted edited/full text");
  await page.getByRole("searchbox", { name: "発言を検索", exact: true }).fill("コピーする");
  await page.getByRole("button", { name: "文字起こしをコピー", exact: true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.copiedText.includes("佐藤"));
  await page.getByRole("searchbox", { name: "発言を検索", exact: true }).fill("");
  const savedCopy = await page.evaluate(() => window.monBrowserFixture.copiedText);
  await page.getByRole("button", { name: "00:00:00の発言を編集", exact: true }).click();
  await page.getByRole("textbox", { name: "00:00:00の発言", exact: true }).fill("保存に失敗する入力");
  await page.evaluate(() => { window.monBrowserFixture.failSave = true; });
  await page.getByRole("button", { name: "文字起こしをコピー", exact: true }).click();
  const saveNotice = page.getByRole("complementary", {name:"通知",exact:true}).getByRole("alert").filter({hasText:"文字起こしの保存:"});
  await saveNotice.waitFor();
  await page.locator(".toast-card").filter({hasText:"文字起こしの保存:"}).getByRole("button", {name:"エラー通知を閉じる",exact:true}).click();
  await page.getByRole("complementary", {name:"通知",exact:true}).getByRole("alert").filter({hasText:"コピーできませんでした:"}).waitFor();
  check(await page.evaluate(() => window.monBrowserFixture.copiedText) === savedCopy, "save failure changed clipboard");
  check(await page.getByRole("textbox", { name: "00:00:00の発言", exact: true }).inputValue() === "保存に失敗する入力", "save failure discarded input");
  // Reload the synthetic data so the failed draft cannot affect control tests.
  await page.reload();
  await page.getByRole("button", { name: "会議A", exact: true }).click();
  for (const kind of ["transcription", "diarization"]) {
    await page.evaluate(async kind => {
      const f = window.monBrowserFixture;
      const job = { ...f.jobs[0], id: `controls-${kind}`, kind, state: "running" };
      f.jobs = [job];
      f.analysisFlow = { id: `flow-${kind}`, meetingId: "meeting-0", stage: kind, lockVersion: 1,
        transcriptionJob: kind === "transcription" ? job : { ...job, id: "completed-asr", kind: "transcription", state: "succeeded" },
        diarizationJob: kind === "diarization" ? job : null, errorMessage: null };
      await f.emitNotification();
    }, kind);
    const panel = page.getByRole("tabpanel", { name: "文字起こし", exact: true });
    await panel.getByRole("button", { name: "生成を一時停止", exact: true }).click();
    await panel.getByRole("button", { name: "生成を再開", exact: true }).click();
    await panel.getByRole("button", { name: "生成を一時停止", exact: true }).waitFor();
    await panel.getByRole("button", { name: "生成を取り消す", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "文字起こし生成を取り消す", exact: true });
    await dialog.getByRole("button", { name: "確認をキャンセル", exact: true }).click();
    check(await page.evaluate(() => window.monBrowserFixture.jobs[0].state) === "running", "dismiss cancelled job");
    await panel.getByRole("button", { name: "生成を取り消す", exact: true }).click();
    await dialog.getByRole("button", { name: "生成を取り消す", exact: true }).click();
    await page.waitForFunction(() => window.monBrowserFixture.jobs[0].state === "cancelled");
    check(await page.evaluate(() => window.monBrowserFixture.currentId) === "analysis", "cancel replaced saved transcript");
    check(await panel.getByRole("button", { name: "生成を一時停止", exact: true }).count() === 0, "terminal job retained controls");
  }
  for (const state of ["queued", "pausing", "interrupted"]) {
    await page.evaluate(async state => {
      const f = window.monBrowserFixture, job = { ...f.jobs[0], id: "state-job", kind: "transcription", state };
      f.jobs = [job];
      f.analysisFlow = { ...f.analysisFlow, stage: "transcription", transcriptionJob: job, diarizationJob: null };
      await f.emitNotification();
    }, state);
    await page.getByRole("button", { name: "生成を取り消す", exact: true }).waitFor();
    const actions = page.locator(".transcript-generation-host");
    await page.waitForFunction(state => {
      const host = document.querySelector(".transcript-generation-host");
      return state === "queued" ? !!host.querySelector('[aria-label="生成を一時停止"]')
        : state === "interrupted" ? host.textContent.includes("生成を再開")
        : !host.querySelector('[aria-label="生成を一時停止"]') && !host.textContent.includes("生成を再開");
    }, state);
    check(await actions.getByRole("button", { name: "生成を取り消す", exact: true }).isEnabled(), `${state} cannot be cancelled`);
  }
  await page.getByRole("button", { name: "生成を取り消す", exact: true }).click();
  const changed = page.getByRole("dialog", { name: "文字起こし生成を取り消す", exact: true });
  const cancelCount = await page.evaluate(() => window.monBrowserFixture.calls.filter(call => call.command === "cancel_analysis_job").length);
  await page.evaluate(() => {
    const f = window.monBrowserFixture;
    f.analysisFlow = { ...f.analysisFlow, stage: "diarization", diarizationJob: { ...f.jobs[0], id: "next-stage-job", state: "running" } };
  });
  await changed.getByRole("button", { name: "生成を取り消す", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "生成の状態が変わりました" }).waitFor();
  check(await page.evaluate(() => window.monBrowserFixture.calls.filter(call => call.command === "cancel_analysis_job").length) === cancelCount, "stale confirmation cancelled a different stage");
  await changed.getByRole("button", { name: "確認を閉じる", exact: true }).click();
  await page.setViewportSize({ width: 760, height: 800 });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "header overflows narrow viewport");
  return {passed:10,copyToasts:true,editedFullCopy:true,saveFailureRetention:true,jobControls:true,staleConfirmation:true,narrowLayout:true};
}
