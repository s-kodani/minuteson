// Synthetic IPC through the production global progress owner.
async page => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  await page.route("**/diarization-progress.html", route => route.fulfill({ contentType: "text/html", body: '<html lang="ja"><div id="root"></div><script type="module" src="/tests/browser-fixture.js"></script></html>' }));
  await page.goto("http://127.0.0.1:1420/diarization-progress.html");
  await page.getByRole("button", { name: "会議A", exact: true }).waitFor();
  const common = page.getByRole("region", { name: "進行中の処理", exact: true });
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    const job = { id: crypto.randomUUID(), meetingId: "meeting-0", meetingTitle: "会議A", operationId: crypto.randomUUID(), kind: "diarization", state: "running", completedUnits: 0, totalUnits: 2, errorCode: null, outputAnalysisId: null, pauseReason: null, specHash: "a".repeat(64), inputHash: "b".repeat(64), retryOfJobId: null, attempt: 1, createdAt: Date.now(), updatedAt: Date.now() };
    f.jobs = [job]; f.notificationJobs = [job];
    f.progressEvent = { jobId: job.id, specHash: job.specHash, attempt: 1, unitIndex: 0, invocationOrder: 1, sequence: 1, engineBasisPoints: 1250, finishing: false };
    await f.emitNotification();
  });
  await common.getByText("0%", { exact: true }).waitFor();
  await page.evaluate(() => window.monBrowserFixture.emitDiarizationRuntime(window.monBrowserFixture.progressEvent));
  await common.getByText("12%", { exact: true }).waitFor();
  check(await common.getByRole("progressbar").getAttribute("aria-valuetext") === "12% · 話者分離 · エンジンで処理中", "accessible percentage missing");
  await common.getByText(/1 \/ 2 処理単位 · この処理単位のエンジン進捗/).waitFor();
  await page.getByRole("button", { name: "設定", exact: true }).click();
  await common.getByText("12%", { exact: true }).waitFor();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture, e = f.progressEvent;
    for (const patch of [{ jobId: "foreign" }, { specHash: "other" }, { attempt: 0 }, { unitIndex: 1 }, { sequence: 0 }, { engineBasisPoints: 10001 }]) await f.emitDiarizationRuntime({ ...e, ...patch });
  });
  await common.getByText("12%", { exact: true }).waitFor();
  await page.evaluate(() => { const f = window.monBrowserFixture; return f.emitDiarizationRuntime({ ...f.progressEvent, sequence: 3, engineBasisPoints: 7500 }); });
  await common.getByText("75%", { exact: true }).waitFor();
  await page.getByRole("button", { name: /ジョブ一覧/ }).click();
  await page.getByRole("heading", { name: "ジョブ一覧", exact: true }).waitFor();
  check(await page.locator("progress[value='75']").count() === 2, "jobs list did not share engine progress with the header");
  await page.evaluate(() => { const f = window.monBrowserFixture; return f.emitDiarizationRuntime({ ...f.progressEvent, sequence: 4, engineBasisPoints: 10000 }); });
  await common.getByText("話者分離 · 結果を照合・保存中", { exact: true }).waitFor();
  check(await common.getByRole("progressbar").getAttribute("value") === null, "engine completion claimed job completion");
  await common.getByText(/エンジン処理 100%/).waitFor();
  for (const button of await page.locator(".toast-host").getByRole("button", { name: /通知を閉じる/ }).all()) await button.click();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: "output/playwright/diarization-progress-narrow.png" });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), "narrow layout overflows");
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.jobs = f.notificationJobs = [{ ...f.jobs[0], state: "paused", updatedAt: f.jobs[0].updatedAt + 1 }];
    await f.emitNotification();
  });
  await common.getByText("話者分離 · 一時停止", { exact: true }).waitFor();
  await common.getByText("0%", { exact: true }).waitFor();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.jobs = f.notificationJobs = [{ ...f.jobs[0], state: "running", completedUnits: 1, updatedAt: f.jobs[0].updatedAt + 1 }];
    await f.emitNotification();
  });
  await common.getByText("50%", { exact: true }).waitFor();
  await page.evaluate(() => { const f = window.monBrowserFixture; return f.emitDiarizationRuntime({ ...f.progressEvent, unitIndex: 1, invocationOrder: 2, sequence: 1, engineBasisPoints: 2500 }); });
  await common.getByText("25%", { exact: true }).waitFor();
  await common.getByText(/2 \/ 2 処理単位/).waitFor();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.jobs = f.notificationJobs = [{ ...f.jobs[0], state: "cancelled", updatedAt: f.jobs[0].updatedAt + 1 }];
    await f.emitNotification();
    await f.emitDiarizationRuntime({ ...f.progressEvent, unitIndex: 1, invocationOrder: 2, sequence: 3, engineBasisPoints: 9900 });
  });
  await common.waitFor({ state: "hidden" });
  return { passed: 5, results: ["engine percent and accessibility", "navigation and shared job list", "stale/foreign events rejected", "100 percent waits for postprocessing and fits 390px", "pause, next unit and cancellation use authoritative state"] };
}
