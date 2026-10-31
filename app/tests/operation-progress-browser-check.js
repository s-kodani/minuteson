// Run through playwright-cli run-code. All long work is synthetic IPC.
async page => {
  const check = (value, message) => { if (!value) throw Error(message); };
  const common = () => page.getByRole("region", { name: "進行中の処理", exact: true });
  const expand = async () => {
    const button = common().locator('.progress-disclosure');
    await button.waitFor();
    if (await button.getAttribute('aria-expanded') !== 'true') await button.click();
  };
  const results = [];
  await page.clock.setFixedTime(new Date("2026-10-17T00:00:00Z"));
  await page.goto("http://127.0.0.1:1420/tests/operation-progress-browser.html");
  await page.getByRole("button", { name: "設定", exact: true }).click();
  const models = page.getByRole("region", { name: "AIモデル", exact: true });
  await models.getByRole("combobox", { name: "文字起こしのモデル", exact: true }).selectOption("official:synthetic-catalog");
  await page.evaluate(() => { window.modelDownloadFixture.mode = "delay"; });
  await models.getByRole("button", { name: "ダウンロードして使用", exact: true }).first().click();
  await page.waitForFunction(() => window.modelDownloadFixture.operations.size === 1);
  await page.clock.setFixedTime(new Date("2026-10-17T00:05:20Z"));
  await page.evaluate(() => window.operationProgressFixture.emitTransfer(58));
  await expand(); await common().getByText(/全体の経過時間 5分20秒/).waitFor();
  await expand(); await common().getByText("50%", { exact: true }).waitFor();
  check(await models.getByRole("progressbar").count() === 0, "model settings duplicates header progress");
  const appearance = await common().locator(".operation-progress").evaluate(element => { const style = getComputedStyle(element); return { background: style.backgroundColor, border: style.borderTopWidth, radius: style.borderRadius, shadow: style.boxShadow, top: element.getBoundingClientRect().top, headerBottom: document.querySelector(".topbar").getBoundingClientRect().bottom, insideHeader: !!element.closest(".topbar") }; });
  check(appearance.background !== "rgba(0, 0, 0, 0)" && appearance.border === "1px" && appearance.radius === "12px" && appearance.shadow !== "none", "floating card lacks rounded background or shadow");
  check(!appearance.insideHeader && appearance.top >= appearance.headerBottom + 10, "progress card is not separated below header");
  const layer = await common().evaluate(element => { const style = getComputedStyle(element); return { position: style.position, background: style.backgroundColor, pointerEvents: style.pointerEvents }; });
  check(layer.position === "absolute" && layer.background === "rgba(0, 0, 0, 0)" && layer.pointerEvents === "none", "progress still paints or occupies an opaque page row");
  check(await common().locator("small").isVisible(), "progress detail is hidden");
  await page.evaluate(async () => {
    await window.operationProgressFixture.emitTransfer(100, { operationId: crypto.randomUUID() });
    await window.operationProgressFixture.emitTransfer(20);
  });
  check(await common().getByText("50%", { exact: true }).count() === 1, "foreign or older progress replaced active download");
  await page.getByRole("button", { name: "会議一覧", exact: true }).last().click();
  await expand(); await common().getByText("50%", { exact: true }).waitFor();
  check(await models.count() === 0, "model panel visible after navigation");
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(call => call.command === "cancel_model_inspection").length) === 0, "navigation cancelled background model preparation");
  await page.evaluate(() => window.operationProgressFixture.emitTransfer(116));
  await expand(); await common().getByText("取得済み · 展開・検証中", { exact: true }).waitFor();
  check(await common().getByRole("progressbar").getAttribute("value") === null, "100% bytes claimed successful model preparation");
  await page.evaluate(() => window.modelDownloadFixture.finish());
  await expand(); await common().getByText("モデルを配置中", { exact: true }).waitFor();
  await expand(); await common().getByText(/全体の経過時間 5分20秒/).waitFor();
  await page.getByRole("button", { name: "設定", exact: true }).click();
  await expand(); await common().getByText("モデルを配置中", { exact: true }).waitFor();
  await expand(); await common().getByText(/全体の経過時間 5分20秒/).waitFor();
  await page.evaluate(() => { window.modelDownloadFixture.mode = "success"; window.modelDownloadFixture.finish(); });
  await common().waitFor({ state: "hidden" });
  await models.getByText("使用中", { exact: true }).waitFor();
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(call => call.command === "start_model_download").length) === 1, "returning to settings restarted download");
  results.push("download percentages, event binding, navigation continuity, verification and apply");

  await page.goto("http://127.0.0.1:1420/tests/operation-progress-browser.html");
  await page.getByRole("button", { name: "設定", exact: true }).click();
  await models.getByRole("combobox", { name: "文字起こしのモデル", exact: true }).selectOption("official:synthetic-catalog");
  await page.evaluate(() => { window.modelDownloadFixture.mode = "delay"; });
  await models.getByRole("button", { name: "ダウンロードして使用", exact: true }).first().click();
  await page.waitForFunction(() => window.modelDownloadFixture.active.size === 1);
  await page.evaluate(() => window.dataFixture.changed({ token: crypto.randomUUID(), kind: "backup", phase: "copying", busy: true }));
  await page.waitForFunction(() => window.modelDownloadFixture.calls.some(call => call.command === "cancel_model_inspection"));
  await page.evaluate(() => window.modelDownloadFixture.finish());
  await expand(); await common().getByText("モデル準備 · 再試行を準備中", { exact: true }).waitFor();
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(call => call.command === "place_downloaded_model").length) === 0, "stopped download proceeded to placement");
  results.push("maintenance still stops preparation and cannot auto-advance it");

  await page.goto("http://127.0.0.1:1420/tests/import-browser.html");
  await page.getByRole("button", { name: "音声を取り込む", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "音声を取り込む", exact: true });
  await dialog.getByRole("button", { name: "取り込む", exact: true }).waitFor();
  await page.waitForFunction(() => window.monImportFixture.import.phase === "ready");
  await page.evaluate(() => window.monImportFixture.changed({ phase: "copying", busy: true, completed: 128022 }));
  await dialog.getByText("50%", { exact: true }).waitFor();
  check(await page.locator(".common-operation-progress progress").getAttribute("value") === "50", "common import progress differs from modal");
  await page.evaluate(() => window.monImportFixture.changed({ phase: "converting", completed: 16000 }));
  await dialog.getByText("25%", { exact: true }).waitFor();
  await page.evaluate(() => window.monImportFixture.changed({ phase: "decoding", completed: 0 }));
  check(await dialog.getByRole("progressbar").getAttribute("value") === null, "decode used a fake percentage");
  await page.evaluate(() => window.monImportFixture.changed({ phase: "cancelling" }));
  await dialog.getByText("安全に取消中", { exact: true }).waitFor();
  await page.evaluate(() => window.monImportFixture.changed({ phase: "cancelled", busy: false }));
  await page.locator(".common-operation-progress").waitFor({ state: "hidden" });
  results.push("import modal/common share bytes and frames; decode and cancellation are indeterminate");

  await page.route("**/progress-analysis.html", route => route.fulfill({ contentType: "text/html", body: '<html lang="ja"><div id="root"></div><script type="module" src="/tests/browser-fixture.js"></script></html>' }));
  await page.goto("http://127.0.0.1:1420/progress-analysis.html");
  await page.getByRole("button", { name: "会議A", exact: true }).waitFor();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    const job = { id: crypto.randomUUID(), meetingId: "meeting-0", meetingTitle: "会議A", operationId: crypto.randomUUID(), kind: "transcription", state: "running", completedUnits: 1, totalUnits: 4, errorCode: null, outputAnalysisId: null, outputMinutesId: null, pauseReason: null, specHash: "a".repeat(64), inputHash: "b".repeat(64), retryOfJobId: null, attempt: 1, createdAt: Date.now() - 320000, updatedAt: Date.now() };
    f.jobs = [job]; f.notificationJobs = [job];
    f.analysisFlow = { id: crypto.randomUUID(), meetingId: "meeting-0", stage: "transcription", lockVersion: 1, transcriptionJob: job, diarizationJob: null, errorMessage: null };
    await f.emitNotification();
  });
  await expand(); await common().getByText("25%", { exact: true }).waitFor();
  await expand(); await common().getByText(/全体の経過時間 5分20秒/).waitFor();
  await page.getByRole("button", { name: "会議A", exact: true }).click();
  check(await page.locator(".meeting-detail-content .operation-progress").count() === 0, "meeting duplicates header progress");
  for (const state of ["paused", "interrupted", "running"]) {
    await page.evaluate(async state => {
      const f = window.monBrowserFixture, job = { ...f.jobs[0], state, completedUnits: 2, updatedAt: f.jobs[0].updatedAt + 1 };
      f.jobs = [job]; f.notificationJobs = [job]; f.analysisFlow.transcriptionJob = job; await f.emitNotification();
    }, state);
    await expand(); await common().getByText("50%", { exact: true }).waitFor();
  }
  await page.getByRole("button", { name: "設定", exact: true }).click();
  await expand(); await common().getByText("50%", { exact: true }).waitFor();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.notificationJobs = [{ ...f.jobs[0], completedUnits: 4, updatedAt: f.jobs[0].updatedAt + 1 }]; await f.emitNotification();
  });
  await expand(); await common().getByText("文字起こし · 結果を保存中", { exact: true }).waitFor();
  check(await common().getByRole("progressbar").getAttribute("value") === null, "final transcription save claimed success");
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    const first = { ...f.notificationJobs[0], state: "cancelled", updatedAt: f.notificationJobs[0].updatedAt + 1 };
    const next = { ...first, id: crypto.randomUUID(), operationId: crypto.randomUUID(), kind: "diarization", state: "running", completedUnits: 0, totalUnits: 2, createdAt: first.updatedAt + 1, updatedAt: first.updatedAt + 1 };
    f.notificationJobs = [next, first]; await f.emitNotification();
  });
  await expand(); await common().getByText("話者分離 · 処理中", { exact: true }).waitFor();
  check(await common().getByRole("progressbar").getAttribute("value") === null, "diarization without runtime progress invented a percentage");
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.notificationJobs = f.notificationJobs.map(job => ({ ...job, state: "failed", errorCode: "synthetic failure", updatedAt: job.updatedAt + 1 })); await f.emitNotification();
  });
  await common().waitFor({ state: "hidden" });
  results.push("transcription header units, no inline duplicate, pause/resume, navigation, finalization, diarization and terminal cleanup");

  await page.evaluate(async () => {
    const f = window.monBrowserFixture, now = Date.now();
    const job = { ...f.notificationJobs[0], id: crypto.randomUUID(), operationId: crypto.randomUUID(), kind: "minutes_generation", state: "running", completedUnits: 0, totalUnits: 10, errorCode: null, createdAt: now - 600000, updatedAt: now };
    f.notificationJobs = [job]; await f.emitNotification();
  });
  await expand(); await common().getByText("議事録生成 · 解析中 0 / 10", { exact: true }).waitFor();
  check(await common().getByRole("progressbar").getAttribute("value") === "0", "minutes did not start at 0%");
  for (const [state, done, label] of [["running", 2, "解析中"], ["paused", 2, "一時停止"], ["running", 7, "解析中"], ["running", 10, "結果を保存中"]]) {
    await page.evaluate(async ({ state, done }) => {
      const f = window.monBrowserFixture;
      f.notificationJobs = [{ ...f.notificationJobs[0], state, completedUnits: done, updatedAt: f.notificationJobs[0].updatedAt + 1 }];
      await f.emitNotification();
    }, { state, done });
    await expand(); await common().getByText(`議事録生成 · ${label} ${done} / 10`, { exact: true }).waitFor();
    await expand(); await common().getByText(`${done * 10}%`, { exact: true }).waitFor();
    check(await common().getByRole("progressbar").getAttribute("value") === String(done * 10), "minutes progress differs from its completed units");
    await expand(); await common().getByText(/全体の経過時間 10分0秒/).waitFor();
    if (state === "paused") check(await common().getByText(/工程目：/).count() === 0, "paused job retained a running phase");
    if (state === "running" && done === 2) {
      await page.evaluate(async () => {
        const f=window.monBrowserFixture, job=f.notificationJobs[0];
        const event={jobId:job.id,specHash:job.specHash,attempt:job.attempt,unitIndex:2,invocation:crypto.randomUUID(),invocationOrder:1,startedAtUtcMs:Date.now(),sequence:1,phase:"input",merge:false,elapsedMs:320000};
        f.runtimeEvent=event; await f.emitMinutesRuntime(event);
      });
      await expand(); await common().getByText(/3工程目：入力を解析中 · 全体の経過時間 10分0秒/).waitFor();
      check(await common().getByRole("progressbar").getAttribute("value")==="20","elapsed time advanced completed progress");
      await common().screenshot({path:"output/playwright/minutes-performance-input.png"});
      await page.evaluate(async () => { const f=window.monBrowserFixture; await f.emitMinutesRuntime({...f.runtimeEvent,sequence:2,phase:"generating",elapsedMs:321000}); });
      await expand(); await common().getByText(/3工程目：本文を生成中/).waitFor();
      await page.evaluate(async () => { const f=window.monBrowserFixture; await f.emitMinutesRuntime({...f.runtimeEvent,sequence:1,elapsedMs:1}); await f.emitMinutesRuntime({...f.runtimeEvent,jobId:"other-job",sequence:99}); });
      check(await common().getByText(/3工程目：本文を生成中/).count()===1,"stale or foreign runtime event replaced current phase");
    }
    if (state === "running" && done === 7) {
      await page.evaluate(async () => {
        const f = window.monBrowserFixture;
        await f.emitMinutesRuntime({ ...f.runtimeEvent, unitIndex: 7, invocation: crypto.randomUUID(), invocationOrder: 2, sequence: 1, phase: "input", elapsedMs: 0 });
      });
      await expand(); await common().getByText(/8工程目：入力を解析中 · 全体の経過時間 10分0秒/).waitFor();
    }
  }
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.notificationJobs = [{ ...f.notificationJobs[0], state: "cancelled", updatedAt: f.notificationJobs[0].updatedAt + 1 }];
    await f.emitNotification();
  });
  await common().waitFor({ state: "hidden" });
  results.push("minutes analysis counts: 0%, 20%, pause, 70%, saving 100% and terminal cleanup");

  await page.evaluate(async () => {
    const f = window.monBrowserFixture, now = Date.now();
    const base = { ...f.notificationJobs[0], id: crypto.randomUUID(), operationId: crypto.randomUUID(), kind: "transcription", meetingId: "meeting-0", meetingTitle: "会議A", state: "running", completedUnits: 1, totalUnits: 4, errorCode: null, createdAt: now, updatedAt: now };
    f.notificationJobs = [base,
      { ...base, id: crypto.randomUUID(), operationId: crypto.randomUUID(), kind: "diarization", state: "queued", completedUnits: 0 },
      { ...base, id: crypto.randomUUID(), operationId: crypto.randomUUID(), kind: "minutes_generation", state: "queued", completedUnits: 0, totalUnits: 1 },
      { ...base, id: crypto.randomUUID(), operationId: crypto.randomUUID(), meetingId: "meeting-1", meetingTitle: "会議B", state: "paused" }];
    await f.emitNotification();
  });
  await page.waitForFunction(() => document.querySelector(".job-nav-count")?.textContent === "4");
  await expand();
  check(await common().getByRole("button", {name: /ジョブ一覧で確認/}).isVisible(), "expanded progress lost the job-list link");
  await page.getByRole("button", { name: "ジョブ一覧", exact: true }).click();
  const jobs = page.getByRole("region", { name: "バックグラウンドのジョブ一覧", exact: true });
  await jobs.locator("li").last().waitFor();
  check(await jobs.locator("li").count() === 4, "job inventory hides waiting or paused work");
  check(await jobs.getByText("待機中", { exact: true }).count() === 2, "waiting states are missing");
  await jobs.getByText("議事録生成", { exact: true }).waitFor();
  check(await jobs.locator(".operation-progress-queued progress").first().evaluate(element => getComputedStyle(element).animationName) === "none", "queued work looks like running work");
  await jobs.getByText("一時停止", { exact: true }).waitFor();
  await jobs.locator("li").first().getByRole("button", { name: "会議を開く", exact: true }).click();
  await page.getByRole("heading", { name: "会議A", exact: true }).waitFor();
  check(await page.locator(".meeting-detail-content .operation-progress").count() === 0, "opening a job restores inline progress");
  await page.getByRole("button", { name: "ジョブ一覧", exact: true }).click();
  await page.evaluate(async () => { window.monBrowserFixture.notificationReadFail = true; await window.monBrowserFixture.emitNotification(); });
  await page.getByRole("status").filter({hasText:"ジョブの状態を確認中"}).waitFor();
  check(await jobs.locator("li").count() === 4, "failed read cleared retained work");
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.notificationReadFail = false;
    f.notificationJobs = f.notificationJobs.map(job => ({ ...job, state: "cancelled", updatedAt: job.updatedAt + 1 }));
    await f.emitNotification();
  });
  await jobs.getByRole("heading", { name: "進行中のジョブはありません", exact: true }).waitFor();
  await common().waitFor({ state: "hidden" });
  results.push("job inventory across meetings, waiting minutes generation, paused work, safe navigation, read errors and empty state");
  return { passed: results.length, results };
}
